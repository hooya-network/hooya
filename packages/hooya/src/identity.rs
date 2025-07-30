use libp2p::identify;
use std::sync::Arc;
use tracing::{event, Level};

pub struct IdentityManager {
    discv5: Arc<discv5::Discv5>,
    awaiting_first_peer: bool,
    bootstrap_peer_id: Option<libp2p::PeerId>,
}

impl IdentityManager {
    pub fn new(discv5: Arc<discv5::Discv5>, needs_update: bool) -> Self {
        Self {
            discv5,
            awaiting_first_peer: needs_update,
            bootstrap_peer_id: None,
        }
    }

    pub fn set_bootstrap_peer_id(&mut self, peer_id: libp2p::PeerId) {
        self.bootstrap_peer_id = Some(peer_id);
    }

    pub fn handle_identify_event(&mut self, event: &identify::Event) {
        event!(Level::DEBUG, awaiting_first_peer = self.awaiting_first_peer, "identity manager received identify event");
        
        if let identify::Event::Received { peer_id, info } = event {
            event!(Level::DEBUG, %peer_id, observed_addr = %info.observed_addr, "processing identify event");
            
            let is_bootstrap = self.bootstrap_peer_id.as_ref() == Some(peer_id);
            let should_update = is_bootstrap || self.awaiting_first_peer;
            
            if should_update {
                if let Some(observed_addr) = extract_socket_addr(&info.observed_addr) {
                    let source = if is_bootstrap { "bootstrap" } else { "first peer" };
                    event!(Level::INFO, %observed_addr, %peer_id, source, "updating ENR with observed address");
                    let _ = self.discv5.update_local_enr_socket(observed_addr, false);
                    self.awaiting_first_peer = false;
                } else {
                    event!(Level::WARN, %peer_id, observed_addr = %info.observed_addr, "failed to extract socket addr from observed address");
                }
            }
        }
    }
}

fn extract_socket_addr(multiaddr: &libp2p::Multiaddr) -> Option<std::net::SocketAddr> {
    let mut ip: Option<std::net::IpAddr> = None;
    let mut port: Option<u16> = None;

    for protocol in multiaddr.iter() {
        match protocol {
            libp2p::multiaddr::Protocol::Ip4(ipv4) => ip = Some(std::net::IpAddr::V4(ipv4)),
            libp2p::multiaddr::Protocol::Ip6(ipv6) => ip = Some(std::net::IpAddr::V6(ipv6)),
            libp2p::multiaddr::Protocol::Tcp(tcp_port) => port = Some(tcp_port),
            libp2p::multiaddr::Protocol::Udp(udp_port) => port = Some(udp_port),
            _ => {}
        }
    }

    match (ip, port) {
        (Some(ip), Some(port)) => Some(std::net::SocketAddr::new(ip, port)),
        _ => None,
    }
}