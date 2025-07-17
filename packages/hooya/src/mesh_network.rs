use crate::mesh::{ChatMessage, MeshMessage};
use anyhow::Result;
use discv5::{Discv5, Enr, Event as Discv5Event};
use futures::StreamExt;
use hooya_config::{DiscoveryConfig, NetworkingConfig};
use libp2p::{
    gossipsub::{
        Behaviour as GossipsubBehaviour, Event as GossipsubEvent, IdentTopic,
    },
    identify::{self, Behaviour as IdentifyBehaviour},
    mdns::{self, tokio::Behaviour as MdnsBehaviour},
    ping::{self, Behaviour as PingBehaviour},
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, PeerId, Swarm,
};
use prost::Message;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use trust_dns_resolver::{config::*, TokioAsyncResolver};

/// Outgoing message types that can be sent over the mesh
#[derive(Debug, Clone)]
pub enum OutgoingMessage {
    Chat {
        channel: String,
        content: String,
        signature: Vec<u8>,
    },
    // Future message types can be added here
    // FileTransfer { ... },
    // StatusUpdate { ... },
}

/// Trait for handling different types of mesh messages
pub trait MessageHandler: Send + Sync {
    /// Validate a mesh message before processing
    fn validate_message(&self, message: &MeshMessage) -> bool;

    /// Handle a validated mesh message synchronously
    /// Handler can use internal channels for async work
    fn handle_message(&self, message: MeshMessage) -> Result<()>;

    /// Get topics this handler subscribes to
    fn get_subscribed_topics(&self) -> Vec<String>;
}

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "MeshBehaviourEvent")]
pub struct MeshBehaviour {
    pub gossipsub: GossipsubBehaviour,
    pub identify: IdentifyBehaviour,
    pub ping: PingBehaviour,
    pub mdns: MdnsBehaviour,
}

#[derive(Debug)]
pub enum MeshBehaviourEvent {
    Gossipsub(GossipsubEvent),
    Identify(identify::Event),
    Ping(ping::Event),
    Mdns(mdns::Event),
}

impl From<GossipsubEvent> for MeshBehaviourEvent {
    fn from(event: GossipsubEvent) -> Self {
        MeshBehaviourEvent::Gossipsub(event)
    }
}

impl From<identify::Event> for MeshBehaviourEvent {
    fn from(event: identify::Event) -> Self {
        MeshBehaviourEvent::Identify(event)
    }
}

impl From<ping::Event> for MeshBehaviourEvent {
    fn from(event: ping::Event) -> Self {
        MeshBehaviourEvent::Ping(event)
    }
}

impl From<mdns::Event> for MeshBehaviourEvent {
    fn from(event: mdns::Event) -> Self {
        MeshBehaviourEvent::Mdns(event)
    }
}

/// Discovery manager handles both mDNS and DNS bootstrap discovery
struct DiscoveryManager {
    mdns_enabled: bool,
    dns_enabled: bool,
    mdns_interval: Duration,
    dns_interval: Duration,
    bootstrap_domain: String,
    last_mdns_discovery: Option<Instant>,
    last_dns_lookup: Option<Instant>,
    dns_resolver: Option<TokioAsyncResolver>,
}

impl DiscoveryManager {
    fn new(config: &DiscoveryConfig) -> Self {
        Self {
            mdns_enabled: config.mdns.enabled,
            dns_enabled: config.dns.enabled,
            mdns_interval: Duration::from_secs(
                config.mdns.discovery_interval_secs,
            ),
            dns_interval: Duration::from_secs(config.dns.lookup_interval_secs),
            bootstrap_domain: config.dns.bootstrap_domain.clone(),
            last_mdns_discovery: None,
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
        let now = Instant::now();
        let mut next_discovery = None;

        if self.mdns_enabled {
            let next_mdns = match self.last_mdns_discovery {
                Some(last) => last + self.mdns_interval,
                None => now, // run immediately if never run
            };

            if next_mdns <= now {
                return Some(Duration::ZERO); // ready to run now
            }

            let mdns_wait = next_mdns - now;
            next_discovery = Some(mdns_wait);
        }

        if self.dns_enabled {
            let next_dns = match self.last_dns_lookup {
                Some(last) => last + self.dns_interval,
                None => now, // run immediately if never run
            };

            if next_dns <= now {
                return Some(Duration::ZERO); // ready to run now
            }

            let dns_wait = next_dns - now;
            next_discovery = Some(
                next_discovery
                    .map_or(dns_wait, |current| current.min(dns_wait)),
            );
        }

        next_discovery
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

        // mDNS discovery is handled by libp2p automatically when enabled
        if self.mdns_enabled
            && self.last_mdns_discovery.is_none_or(|last| {
                now.duration_since(last) >= self.mdns_interval
            })
        {
            // mDNS discovery is passive through libp2p mdns behaviour
            self.last_mdns_discovery = Some(now);
        }

        // DNS bootstrap discovery
        if self.dns_enabled
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

        let txt_records =
            match resolver.txt_lookup(&self.bootstrap_domain).await {
                Ok(records) => records,
                Err(e) => {
                    eprintln!("Failed to lookup DNS bootstrap records: {e}");
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

                if let Err(e) = discv5.add_enr(enr) {
                    eprintln!("Failed to add ENR from DNS: {e}");
                }
            }
        }
    }
}

pub struct MeshNetwork {
    node_id: String,
    known_peers: HashSet<PeerId>,
    networking_config: NetworkingConfig,
    handlers: Vec<Arc<dyn MessageHandler>>,
}

impl MeshNetwork {
    pub fn new(node_id: String, networking_config: NetworkingConfig) -> Self {
        Self {
            node_id,
            known_peers: HashSet::new(),
            networking_config,
            handlers: Vec::new(),
        }
    }

    /// Add a message handler to the network
    pub fn add_handler(&mut self, handler: Arc<dyn MessageHandler>) {
        self.handlers.push(handler);
    }

    pub async fn run(
        &mut self,
        discv5: Discv5,
        mut swarm: Swarm<MeshBehaviour>,
        message_rx: mpsc::Receiver<OutgoingMessage>,
    ) -> Result<()> {
        self.setup_subscriptions(&mut swarm)?;

        let mut discovery_manager =
            DiscoveryManager::new(&self.networking_config.discovery);
        discovery_manager.initialize().await?;

        let discv5_events = self.setup_discv5_events(&discv5).await?;

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
        swarm: &mut Swarm<MeshBehaviour>,
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
        mut swarm: Swarm<MeshBehaviour>,
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
                    self.handle_swarm_event(swarm_event).await;
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
        swarm: &mut Swarm<MeshBehaviour>,
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
                    if !self.known_peers.contains(&peer_id)
                        && swarm.connected_peers().count()
                            < self.networking_config.max_peers
                    {
                        println!("Discovered new peer via discv5: {peer_id}");
                        self.known_peers.insert(peer_id);

                        // attempt to dial
                        if let Err(e) = swarm.dial(multiaddr.clone()) {
                            eprintln!("Failed to dial peer {multiaddr}: {e}");
                        }
                    }
                }
            }
        }
    }

    async fn handle_swarm_event(
        &mut self,
        event: SwarmEvent<MeshBehaviourEvent>,
    ) {
        match event {
            SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(
                gossipsub_event,
            )) => {
                self.handle_gossipsub_event(gossipsub_event).await;
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Identify(
                identify_event,
            )) => {
                self.handle_identify_event(identify_event).await;
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Ping(_ping_event)) => {
                // handle ping events if needed
            }
            SwarmEvent::Behaviour(MeshBehaviourEvent::Mdns(mdns_event)) => {
                self.handle_mdns_event(mdns_event).await;
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                println!("Connected to peer: {peer_id}");
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                println!("Disconnected from peer: {peer_id}");
                self.known_peers.remove(&peer_id);
            }
            _ => {}
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
                eprintln!(
                    "Dropped oversized message: {} bytes > {} bytes limit",
                    message.data.len(),
                    self.networking_config.max_message_size_bytes
                );
                return;
            }

            match MeshMessage::decode(&*message.data) {
                Ok(mesh_msg) => {
                    if !self.validate_mesh_message(&mesh_msg) {
                        eprintln!("Dropped invalid mesh message");
                        return;
                    }

                    for handler in &self.handlers {
                        if handler.validate_message(&mesh_msg) {
                            if let Err(e) =
                                handler.handle_message(mesh_msg.clone())
                            {
                                eprintln!(
                                    "Handler failed to process message: {e}"
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to decode mesh message: {e}");
                }
            }
        }
    }

    async fn handle_identify_event(&self, event: identify::Event) {
        if let identify::Event::Received { peer_id, info: _ } = event {
            println!("Identified peer: {peer_id}");
        }
    }

    async fn handle_mdns_event(&mut self, event: mdns::Event) {
        match event {
            mdns::Event::Discovered(list) => {
                for (peer_id, _multiaddr) in list {
                    if !self.known_peers.contains(&peer_id) {
                        println!("Discovered mDNS peer: {peer_id}");
                        self.known_peers.insert(peer_id);
                    }
                }
            }
            mdns::Event::Expired(list) => {
                for (peer_id, _) in list {
                    println!("mDNS peer expired: {peer_id}");
                    self.known_peers.remove(&peer_id);
                }
            }
        }
    }

    async fn handle_outgoing_message(
        &self,
        outgoing_msg: OutgoingMessage,
        swarm: &mut Swarm<MeshBehaviour>,
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
                    eprintln!("Failed to publish message: {e}");
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
