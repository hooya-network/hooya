use crate::addr_book::AddrBook;
use crate::mesh::{ChatMessage, MeshMessage};
use anyhow::Result;
use discv5::{Discv5, Enr, Event as Discv5Event};
use futures::StreamExt;
use hooya_config::NetworkingConfig;
use libp2p::{
    gossipsub::{
        Behaviour as GossipsubBehavior, Event as GossipsubEvent, IdentTopic,
    },
    identify::{self, Behaviour as IdentifyBehavior},
    mdns::{self, tokio::Behaviour as MdnsBehavior}, // lol american spelling
    ping::{self, Behaviour as PingBehavior},
    swarm::{behaviour::toggle::Toggle, NetworkBehaviour, SwarmEvent},
    Multiaddr,
    PeerId,
    Swarm,
};
use prost::Message;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{event, warn, Level};

/// Outgoing message types that can be sent over the mesh
#[derive(Debug, Clone)]
pub enum OutgoingMessage {
    Chat {
        channel: String,
        content: String,
        // TODO(wesl-ee) this should probably be on all messages
        signature: Vec<u8>,
        pubkey: Vec<u8>,
    },
}

/// Trait for handling different types of mesh messages
pub trait MessageHandler: Send + Sync {
    /// validate a mesh message before processing
    fn validate_message(&self, message: &MeshMessage) -> bool;

    /// handle a validated mesh message synchronously
    /// handler can use internal channels for async work
    fn handle_message(&self, message: MeshMessage) -> Result<()>;

    /// get topics this handler subscribes to
    fn get_subscribed_topics(&self) -> Vec<String>;
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "MeshBehaviorEvent")]
pub struct MeshBehavior {
    pub gossipsub: GossipsubBehavior,
    pub identify: IdentifyBehavior,
    pub ping: PingBehavior,
    pub mdns: Toggle<MdnsBehavior>,
}

#[derive(Debug)]
pub enum MeshBehaviorEvent {
    Gossipsub(GossipsubEvent),
    Identify(Box<identify::Event>),
    Ping(ping::Event),
    Mdns(mdns::Event),
}

impl From<GossipsubEvent> for MeshBehaviorEvent {
    fn from(event: GossipsubEvent) -> Self {
        MeshBehaviorEvent::Gossipsub(event)
    }
}

impl From<identify::Event> for MeshBehaviorEvent {
    fn from(event: identify::Event) -> Self {
        MeshBehaviorEvent::Identify(Box::new(event))
    }
}

impl From<ping::Event> for MeshBehaviorEvent {
    fn from(event: ping::Event) -> Self {
        MeshBehaviorEvent::Ping(event)
    }
}

impl From<mdns::Event> for MeshBehaviorEvent {
    fn from(event: mdns::Event) -> Self {
        MeshBehaviorEvent::Mdns(event)
    }
}

pub struct MeshNetwork {
    node_id: String,
    addr_book: AddrBook,
    networking_config: NetworkingConfig,
    handlers: Vec<Arc<dyn MessageHandler>>,
}

impl MeshNetwork {
    pub fn new(
        node_id: String,
        networking_config: NetworkingConfig,
        addr_book: AddrBook,
    ) -> Self {
        Self {
            node_id,
            addr_book,
            networking_config,
            handlers: Vec::new(),
        }
    }

    /// Add a message handler to the network
    pub fn add_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.handlers.push(handler);
    }

    /// attempt to reconnect to previously known peers on startup
    async fn reconnect_to_known_peers(
        &mut self,
        swarm: &mut Swarm<MeshBehavior>,
    ) {
        let dialable_peers = self.addr_book.get_dialable_peers();

        for (peer_id, multiaddr) in dialable_peers {
            if swarm.connected_peers().count()
                >= self.networking_config.max_peers
            {
                break;
            }

            event!(Level::INFO, %peer_id, %multiaddr, "attempting to reconnect to known peer");
            if let Err(e) = swarm.dial(multiaddr.clone()) {
                event!(Level::WARN, %e, %peer_id, %multiaddr, "failed to dial known peer");
                self.addr_book.record_connection_attempt(&peer_id, false);
            }
        }
    }

    pub async fn run(
        &mut self,
        discv5: Discv5,
        mut swarm: Swarm<MeshBehavior>,
        message_rx: mpsc::Receiver<OutgoingMessage>,
    ) -> Result<()> {
        self.setup_subscriptions(&mut swarm)?;

        let discv5_events = self.setup_discv5_events(&discv5).await?;

        // perform startup address discovery via PING/PONG and wait for SocketUpdated
        self.discovery_phase(&discv5, discv5_events).await?;

        // get fresh event stream after discovery
        let discv5_events = self.setup_discv5_events(&discv5).await?;

        // attempt to reconnect to known peers from previous sessions
        self.reconnect_to_known_peers(&mut swarm).await;

        self.main_event_loop(swarm, discv5, discv5_events, message_rx)
            .await
    }

    fn setup_subscriptions(
        &self,
        swarm: &mut Swarm<MeshBehavior>,
    ) -> Result<()> {
        for handler in &self.handlers {
            for topic_str in handler.get_subscribed_topics() {
                let topic = IdentTopic::new(topic_str);
                swarm.behaviour_mut().gossipsub.subscribe(&topic)?;
            }
        }
        Ok(())
    }

    async fn setup_discv5_events(
        &self,
        discv5: &Discv5,
    ) -> Result<mpsc::Receiver<Discv5Event>> {
        let event_stream = discv5.event_stream().await.map_err(|e| {
            anyhow::anyhow!("Failed to get discv5 event stream: {}", e)
        })?;
        Ok(event_stream)
    }

    async fn main_event_loop(
        &mut self,
        mut swarm: Swarm<MeshBehavior>,
        discv5: Discv5,
        mut discv5_events: mpsc::Receiver<Discv5Event>,
        mut message_rx: mpsc::Receiver<OutgoingMessage>,
    ) -> Result<()> {
        // setup periodic timers
        let mut discovery_interval =
            tokio::time::interval(std::time::Duration::from_secs(
                self.networking_config.discovery.discovery_interval_secs,
            ));
        discovery_interval
            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // p2p info every 60s
        let mut logging_interval =
            tokio::time::interval(std::time::Duration::from_secs(60));
        logging_interval
            .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // this will run one round of peer discovery as soon as
        self.handle_periodic_discovery(
            &discv5,
            swarm.connected_peers().count(),
        )
        .await;

        loop {
            tokio::select! {

                // Handle periodic peer discovery
                _ = discovery_interval.tick() => {
                    self.handle_periodic_discovery(&discv5, swarm.connected_peers().count()).await;
                }

                // Handle periodic logging
                _ = logging_interval.tick() => {
                    self.handle_periodic_logging(swarm.connected_peers().count());
                }

                // Handle discv5 peer discovery
                Some(discv5_event) = discv5_events.recv() => {
                    self.handle_discv5_event(discv5_event, &mut swarm, &discv5).await;
                }

                // Handle libp2p swarm events
                swarm_event = swarm.select_next_some() => {
                    match swarm_event {
                        SwarmEvent::Behaviour(MeshBehaviorEvent::Gossipsub(gossipsub_event)) => {
                            self.handle_gossipsub_event(gossipsub_event).await;
                        }
                        SwarmEvent::Behaviour(MeshBehaviorEvent::Identify(identify_event)) => {
                            self.handle_identify_event(*identify_event, &discv5).await;
                        }
                        SwarmEvent::Behaviour(MeshBehaviorEvent::Mdns(mdns_event)) => {
                            self.handle_mdns_event(mdns_event, &mut swarm).await;
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            event!(Level::INFO, %peer_id, "peer connected");
                            self.addr_book.record_connection_attempt(&peer_id, true);
                        }
                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            event!(Level::INFO, %peer_id, "peer disconnected");
                            self.addr_book.remove_peer(&peer_id).await;
                        }
                        _ => {}
                    }
                }

                // Handle outgoing messages
                Some(outgoing_msg) = message_rx.recv() => {
                    self.handle_outgoing_message(outgoing_msg, &mut swarm).await;
                }
            }
        }
    }

    async fn handle_discv5_event(
        &mut self,
        event: Discv5Event,
        swarm: &mut Swarm<MeshBehavior>,
        discv5: &Discv5,
    ) {
        event!(Level::DEBUG, ?event, "received discv5 event");

        match event {
            Discv5Event::NodeInserted {
                node_id,
                replaced: _,
            } => {
                // get the ENR for this node from discv5
                if let Some(enr) = discv5.find_enr(&node_id) {
                    self.handle_discovered_enr(
                        enr,
                        swarm,
                        "discv5 node inserted",
                    )
                    .await;
                }
            }
            Discv5Event::SessionEstablished(enr, _socket_addr) => {
                event!(Level::DEBUG, enr = %enr, "discv5 session established");
                self.handle_discovered_enr(
                    enr,
                    swarm,
                    "discv5 session established",
                )
                .await;
            }
            Discv5Event::Discovered(enr) => {
                event!(Level::DEBUG, enr = %enr, "discv5 peer discovered");
                self.handle_discovered_enr(enr, swarm, "discv5 discovered")
                    .await;
            }
            _ => {
                // handle other events if needed
            }
        }
    }

    async fn handle_discovered_enr(
        &mut self,
        enr: Enr,
        swarm: &mut Swarm<MeshBehavior>,
        source: &str,
    ) {
        if let Some(multiaddr) = self.enr_to_multiaddr(&enr) {
            let peer_id = self.enr_to_peer_id(&enr);

            // only connect if we don't already know this peer
            if !self.addr_book.contains_peer(&peer_id)
                && swarm.connected_peers().count()
                    < self.networking_config.max_peers
            {
                event!(Level::INFO, %peer_id, %multiaddr, source, "discovered new peer");
                self.addr_book.add_peer_with_enr(peer_id, enr.clone()).await;

                // attempt to dial
                if let Err(e) = swarm.dial(multiaddr.clone()) {
                    event!(Level::WARN, %e, %peer_id, %multiaddr, "failed to dial discovered peer");
                    self.addr_book.record_connection_attempt(&peer_id, false);
                }
            }
        }
    }

    async fn handle_gossipsub_event(&self, event: GossipsubEvent) {
        if let GossipsubEvent::Message {
            message,
            message_id: _,
            propagation_source: _,
        } = event
        {
            if message.data.len()
                > self.networking_config.max_message_size_bytes
            {
                event!(
                    Level::WARN,
                    message_size = message.data.len(),
                    size_limit = self.networking_config.max_message_size_bytes,
                    "dropped oversized message"
                );
                return;
            }

            match MeshMessage::decode(&*message.data) {
                Ok(mesh_msg) => {
                    if !self.validate_mesh_message(&mesh_msg) {
                        event!(Level::DEBUG, "dropped invalid mesh message");
                        return;
                    }

                    for handler in &self.handlers {
                        if handler.validate_message(&mesh_msg) {
                            if let Err(e) =
                                handler.handle_message(mesh_msg.clone())
                            {
                                event!(Level::ERROR, %e, "handler failed to process message");
                            }
                        }
                    }
                }
                Err(e) => {
                    event!(Level::WARN, %e, "failed to decode mesh message");
                }
            }
        }
    }

    async fn handle_identify_event(
        &self,
        event: identify::Event,
        discv5: &Discv5,
    ) {
        let identify::Event::Received {
            peer_id,
            info,
            connection_id: _,
        } = event
        else {
            return;
        };

        event!(Level::DEBUG, %peer_id, "identified peer");

        // only update our ENR if mDNS is enabled and we have no external address yet
        if !self.networking_config.discovery.mdns.enabled {
            return;
        }

        let local_enr = discv5.local_enr();
        if local_enr.ip4().is_some() || local_enr.ip6().is_some() {
            return; // already have an address
        }

        // extract our external IP from the observed address in identify info
        let ip = match crate::address_discovery::extract_ip_from_multiaddr(
            &info.observed_addr,
        ) {
            Some(ip) => ip,
            None => return,
        };

        // get our discv5 UDP port from config
        let (ipv4_config, ipv6_config) =
            match self.networking_config.get_discv5_addresses() {
                Ok(config) => config,
                Err(_) => return,
            };

        let udp_port = match ip {
            std::net::IpAddr::V4(_) => ipv4_config.map(|(_, port)| port),
            std::net::IpAddr::V6(_) => ipv6_config.map(|(_, port)| port),
        };

        let udp_port = match udp_port {
            Some(port) => port,
            None => return,
        };

        let socket = SocketAddr::new(ip, udp_port);
        discv5.update_local_enr_socket(socket, false);
        let enr = discv5.local_enr();
        event!(Level::INFO, %enr, "updated enr after mdns identify");
    }

    async fn handle_mdns_event(
        &mut self,
        event: mdns::Event,
        swarm: &mut Swarm<MeshBehavior>,
    ) {
        match event {
            mdns::Event::Discovered(list) => {
                for (peer_id, multiaddr) in list {
                    if !self.addr_book.contains_peer(&peer_id)
                        && swarm.connected_peers().count()
                            < self.networking_config.max_peers
                    {
                        event!(Level::INFO, %peer_id, %multiaddr, "discovered mDNS peer");

                        // dial the discovered peer directly without adding to addr_book
                        // since mDNS doesn't provide ENRs, we can't store them properly
                        if let Err(e) = swarm.dial(multiaddr.clone()) {
                            event!(Level::WARN, %e, %peer_id, %multiaddr, "failed to dial mDNS peer");
                        }
                    }
                }
            }
            mdns::Event::Expired(list) => {
                for (peer_id, _) in list {
                    event!(Level::DEBUG, %peer_id, "mDNS peer expired");
                    // Note: mDNS peers aren't stored in addr_book anymore
                }
            }
        }
    }

    async fn handle_outgoing_message(
        &self,
        outgoing_msg: OutgoingMessage,
        swarm: &mut Swarm<MeshBehavior>,
    ) {
        match outgoing_msg {
            OutgoingMessage::Chat {
                channel,
                content,
                signature,
                pubkey,
            } => {
                let topic = IdentTopic::new(format!("/chat/room/{channel}"));

                let chat_msg = ChatMessage {
                    channel: channel.clone(),
                    content: content.clone(),
                };

                let mesh_msg = MeshMessage {
                    version: 1,
                    timestamp: chrono::Utc::now().timestamp() as u64,
                    sender_node_id: self.node_id.clone(),
                    signature: signature.clone(),
                    pubkey: pubkey.clone(),
                    payload: Some(crate::mesh::mesh_message::Payload::Chat(
                        chat_msg,
                    )),
                };

                let encoded = mesh_msg.encode_to_vec();
                if let Err(e) =
                    swarm.behaviour_mut().gossipsub.publish(topic, encoded)
                {
                    event!(Level::ERROR, %e, "failed to publish message");
                }
            }
        }
    }

    /// Convert ENR to multiaddr for libp2p connection
    fn enr_to_multiaddr(&self, enr: &Enr) -> Option<Multiaddr> {
        let mut multiaddr = Multiaddr::empty();

        // extract IP address
        if let Some(ip4) = enr.ip4() {
            multiaddr.push(libp2p::multiaddr::Protocol::Ip4(ip4));
        } else if let Some(ip6) = enr.ip6() {
            multiaddr.push(libp2p::multiaddr::Protocol::Ip6(ip6));
        } else {
            return None;
        }

        // extract TCP port (prefer tcp4, fallback to tcp6)
        if let Some(tcp_port) = enr.tcp4().or(enr.tcp6()) {
            multiaddr.push(libp2p::multiaddr::Protocol::Tcp(tcp_port));
        } else {
            return None;
        }

        Some(multiaddr)
    }

    /// Extract PeerId from ENR
    fn enr_to_peer_id(&self, enr: &Enr) -> PeerId {
        crate::peer_id::enr_to_peer_id(enr)
    }

    /// Validate basic mesh message structure
    fn validate_mesh_message(&self, mesh_msg: &MeshMessage) -> bool {
        // check version compatibility
        if mesh_msg.version == 0 || mesh_msg.version > 1 {
            return false;
        }

        // validate timestamp (not too far in past/future)
        let now = chrono::Utc::now().timestamp() as u64;
        let msg_time = mesh_msg.timestamp;

        // allow messages from 1 hour in past to 5 minutes in future
        if msg_time < now.saturating_sub(3600) || msg_time > now + 300 {
            return false;
        }

        // validate sender node id
        if mesh_msg.sender_node_id.is_empty()
            || mesh_msg.sender_node_id.len() > 64
        {
            return false;
        }

        // verify signature using pubkey
        if let Some(payload) = &mesh_msg.payload {
            match payload {
                crate::mesh::mesh_message::Payload::Chat(chat_msg) => {
                    let message_to_verify =
                        format!("{}:{}", chat_msg.channel, chat_msg.content);
                    match crate::keys::verify_signature(
                        &mesh_msg.pubkey,
                        message_to_verify.as_bytes(),
                        &mesh_msg.signature,
                    ) {
                        Ok(valid) => {
                            if !valid {
                                return false;
                            }
                        }
                        Err(_) => {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    async fn handle_periodic_discovery(
        &mut self,
        discv5: &Discv5,
        connected_count: usize,
    ) {
        // only do discovery if we're below max_peers
        if connected_count >= self.networking_config.max_peers {
            return;
        }

        // query discv5 with random node id to populate routing table
        let random_target = discv5::enr::NodeId::random();
        match discv5.find_node(random_target).await {
            Ok(discovered_enrs) => {
                let mut new_peers = 0;
                for enr in discovered_enrs.iter().take(20) {
                    let peer_id = self.enr_to_peer_id(enr);
                    if !self.addr_book.contains_peer(&peer_id) {
                        self.addr_book
                            .add_peer_with_enr(peer_id, enr.clone())
                            .await;
                        new_peers += 1;
                    }
                }
                if new_peers > 0 {
                    event!(
                        Level::DEBUG,
                        new_peers,
                        "periodic discovery found new peers"
                    );
                }
            }
            Err(e) => {
                event!(Level::DEBUG, %e, "periodic discovery query failed");
            }
        }
    }

    fn handle_periodic_logging(&self, connected_count: usize) {
        let total_known = self.addr_book.get_peers().len();
        event!(
            Level::INFO,
            connected_peers = connected_count,
            known_peers = total_known,
            max_peers = self.networking_config.max_peers,
            "mesh network status"
        );
    }

    /// discovery phase: ping bootstrap nodes and wait for SocketUpdated event
    async fn discovery_phase(
        &mut self,
        discv5: &Discv5,
        _discv5_events: mpsc::Receiver<Discv5Event>,
    ) -> Result<()> {
        event!(Level::INFO, "starting address discovery phase");

        // wait for SocketUpdated event or timeout after reasonable period
        let _discovery_timeout = tokio::time::Duration::from_secs(30);

        // try once for bootstrap discovery
        if let Some((ip, port)) = self.perform_startup_discovery(discv5).await {
            let socket = SocketAddr::new(ip, port);
            discv5.update_local_enr_socket(socket, false);
        } else if self.networking_config.discovery.mdns.enabled {
            // if mDNS is enabled and no bootstrap peers, skip the blocking loop
            // and let mDNS + identify handle address discovery in the main event loop
            event!(Level::INFO, "no bootstrap peers available, relying on mDNS for address discovery");
        } else {
            // no mDNS and no bootstrap peers - this is a problem
            return Err(anyhow::anyhow!("No discovery method available: no bootstrap peers and mDNS disabled"));
        }

        Ok(())
    }

    async fn perform_startup_discovery(
        &self,
        discv5: &Discv5,
    ) -> Option<(IpAddr, u16)> {
        let bootstrap_peers =
            match crate::address_discovery::get_bootstrap_peers(
                &self.addr_book,
                &self.networking_config.discovery,
            )
            .await
            {
                Ok(peers) => peers,
                Err(e) => {
                    event!(Level::WARN, %e, "failed to get bootstrap peers for discovery");
                    return None;
                }
            };

        if bootstrap_peers.is_empty() {
            event!(
                Level::INFO,
                "no bootstrap peers available for startup discovery"
            );
            return None;
        }

        // ping the first available bootstrap peer to discover our external address
        for (peer_id, _multiaddr) in bootstrap_peers.iter().take(3) {
            // convert peer_id back to node_id for discv5 ping
            if let Some(peer_info) = self.addr_book.get_peers().get(peer_id) {
                let enr = peer_info.enr.clone();
                let node_id = enr.node_id();
                event!(Level::INFO, %peer_id, %node_id, "pinging bootstrap peer for address discovery");
                if let Err(e) = discv5.add_enr(enr.clone()) {
                    warn!("failed to add enr to routing table: {}", e);
                }

                match discv5.send_ping(enr).await {
                    Ok(pong) => {
                        let ip = pong.ip;
                        let port = pong.port;
                        event!(Level::INFO, %ip, %port, "discovered external address");
                        return Some((ip, port));
                    }
                    Err(e) => {
                        event!(Level::WARN, %e, %peer_id, "failed to query bootstrap peer");
                    }
                }
            }
        }

        None
    }
}
