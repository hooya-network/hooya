use crate::addr_book::AddrBook;
use crate::mesh::{ChatMessage, MeshMessage};
use anyhow::Result;
use discv5::{Discv5, Enr, Event as Discv5Event};
use futures::StreamExt;
use hooya_config::{DiscoveryConfig, NetworkingConfig};
use libp2p::{
    gossipsub::{
        Behaviour as GossipsubBehavior, Event as GossipsubEvent, IdentTopic,
    },
    identify::{self, Behaviour as IdentifyBehavior},
    mdns::{self, tokio::Behaviour as MdnsBehavior}, // lol american spelling
    ping::{self, Behaviour as PingBehavior},
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr,
    PeerId,
    Swarm,
};
use prost::Message;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{event, Level};
use trust_dns_resolver::{config::*, TokioAsyncResolver};

/// Outgoing message types that can be sent over the mesh
#[derive(Debug, Clone)]
pub enum OutgoingMessage {
    Chat {
        channel: String,
        content: String,
        // TODO(wesl-ee) this should probably be on all messages
        signature: Vec<u8>,
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
    pub mdns: MdnsBehavior,
}

#[derive(Debug)]
pub enum MeshBehaviorEvent {
    Gossipsub(GossipsubEvent),
    Identify(identify::Event),
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
        MeshBehaviorEvent::Identify(event)
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

/// Discovery manager handles DNS bootstrap discovery
struct DiscoveryManager {
    dns_enabled: bool,
    dns_interval: Duration,
    bootstrap_domain: String,
    last_dns_lookup: Option<Instant>,
    dns_resolver: Option<TokioAsyncResolver>,
}

impl DiscoveryManager {
    fn new(config: &DiscoveryConfig) -> Self {
        Self {
            dns_enabled: config.dns.enabled,
            dns_interval: Duration::from_secs(config.dns.lookup_interval_secs),
            bootstrap_domain: config.dns.bootstrap_domain.clone(),
            last_dns_lookup: None,
            dns_resolver: None,
        }
    }

    async fn initialize(&mut self) -> Result<()> {
        if self.dns_enabled {
            let resolver = TokioAsyncResolver::tokio(
                ResolverConfig::default(),
                ResolverOpts::default(),
            );
            self.dns_resolver = Some(resolver);
        }
        Ok(())
    }

    fn time_until_next_discovery(&self) -> Option<Duration> {
        if !self.dns_enabled {
            return None;
        }

        let now = Instant::now();
        let next_dns = match self.last_dns_lookup {
            Some(last) => last + self.dns_interval,
            None => now, // run immediately if never run
        };

        if next_dns <= now {
            Some(Duration::ZERO) // ready to run now
        } else {
            Some(next_dns - now)
        }
    }

    async fn run_discovery_cycle(
        &mut self,
        discv5: &Discv5,
        current_peer_count: usize,
        max_peers: usize,
    ) {
        if current_peer_count >= max_peers {
            return; // no need to discover more peers
        }

        let now = Instant::now();

        // DNS bootstrap discovery - only if we have no connected peers
        if self.dns_enabled
            && current_peer_count == 0
            && self.last_dns_lookup.is_none_or(|last| {
                now.duration_since(last) >= self.dns_interval
            })
        {
            self.discover_dns_bootstrap_peers(discv5).await;
            self.last_dns_lookup = Some(now);
        }
    }

    async fn discover_dns_bootstrap_peers(&self, discv5: &Discv5) {
        let Some(resolver) = &self.dns_resolver else {
            return;
        };

        let txt_records = match resolver
            .txt_lookup(&self.bootstrap_domain)
            .await
        {
            Ok(records) => records,
            Err(e) => {
                event!(Level::WARN, %e, bootstrap_domain = %self.bootstrap_domain, "failed to lookup DNS bootstrap records");
                return;
            }
        };

        for record in txt_records.iter() {
            for txt_data in record.iter() {
                let txt_str = match std::str::from_utf8(txt_data) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                if !txt_str.starts_with("enr:") {
                    continue;
                }

                let enr = match txt_str.parse::<Enr>() {
                    Ok(enr) => enr,
                    Err(_) => continue,
                };

                if let Err(e) = discv5.add_enr(enr.clone()) {
                    event!(Level::WARN, %e, %enr, "failed to add ENR from DNS");
                }
            }
        }
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

        let mut discovery_manager =
            DiscoveryManager::new(&self.networking_config.discovery);
        discovery_manager.initialize().await?;

        let discv5_events = self.setup_discv5_events(&discv5).await?;

        // attempt to reconnect to known peers from previous sessions
        self.reconnect_to_known_peers(&mut swarm).await;

        self.main_event_loop(
            swarm,
            discv5,
            discv5_events,
            message_rx,
            discovery_manager,
        )
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
        mut discovery_manager: DiscoveryManager,
    ) -> Result<()> {
        loop {
            let discovery_timeout = discovery_manager
                .time_until_next_discovery()
                .unwrap_or(Duration::from_secs(3600)); // default to 1 hour if no discovery needed

            tokio::select! {
                // Discovery cycle
                _ = tokio::time::sleep(discovery_timeout) => {
                    discovery_manager.run_discovery_cycle(&discv5, swarm.connected_peers().count(), self.networking_config.max_peers).await;
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
                            self.handle_identify_event(identify_event).await;
                        }
                        SwarmEvent::Behaviour(MeshBehaviorEvent::Ping(_ping_event)) => {
                            // handle ping events if needed
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
        if let Discv5Event::NodeInserted {
            node_id,
            replaced: _,
        } = event
        {
            // get the ENR for this node from discv5
            if let Some(enr) = discv5.find_enr(&node_id) {
                if let Some(multiaddr) = self.enr_to_multiaddr(&enr) {
                    let peer_id = self.enr_to_peer_id(&enr);

                    // only connect if we don't already know this peer
                    if !self.addr_book.contains_peer(&peer_id)
                        && swarm.connected_peers().count()
                            < self.networking_config.max_peers
                    {
                        event!(Level::INFO, %peer_id, %multiaddr, "discovered new peer via discv5");
                        self.addr_book
                            .add_peer(peer_id, multiaddr.clone())
                            .await;

                        // attempt to dial
                        if let Err(e) = swarm.dial(multiaddr.clone()) {
                            event!(Level::WARN, %e, %peer_id, %multiaddr, "failed to dial discovered peer");
                            self.addr_book
                                .record_connection_attempt(&peer_id, false);
                        }
                    }
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

    async fn handle_identify_event(&self, event: identify::Event) {
        if let identify::Event::Received { peer_id, info: _ } = event {
            event!(Level::DEBUG, %peer_id, "identified peer");
        }
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
                        self.addr_book
                            .add_peer(peer_id, multiaddr.clone())
                            .await;

                        // dial the discovered peer to establish connection
                        if let Err(e) = swarm.dial(multiaddr.clone()) {
                            event!(Level::WARN, %e, %peer_id, %multiaddr, "failed to dial mDNS peer");
                            self.addr_book
                                .record_connection_attempt(&peer_id, false);
                        }
                    }
                }
            }
            mdns::Event::Expired(list) => {
                for (peer_id, _) in list {
                    event!(Level::DEBUG, %peer_id, "mDNS peer expired");
                    self.addr_book.remove_peer(&peer_id).await;
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
        // use the node_id as a basis for peer_id
        // in a real implementation you'd extract the actual public key
        let node_id = enr.node_id();
        PeerId::from_bytes(&node_id.raw()).unwrap_or_else(|_| {
            // fallback: generate peer_id from node_id hash
            let mut hasher = DefaultHasher::new();
            node_id.raw().hash(&mut hasher);
            let hash = hasher.finish();
            PeerId::from_bytes(&hash.to_le_bytes()).unwrap_or(PeerId::random())
        })
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

        true
    }
}
