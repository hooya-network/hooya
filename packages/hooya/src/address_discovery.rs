use crate::addr_book::AddrBook;
use anyhow::Result;
use discv5::Enr;
use hooya_config::DiscoveryConfig;
use libp2p::multiaddr::Protocol;
use libp2p::{
    identify::{self, Behaviour as IdentifyBehaviour},
    swarm::NetworkBehaviour,
    Multiaddr, PeerId,
};
use tracing::{event, Level};
use trust_dns_resolver::TokioAsyncResolver;

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "DiscoveryBehaviourEvent")]
pub struct DiscoveryBehaviour {
    pub identify: IdentifyBehaviour,
}

#[derive(Debug)]
pub enum DiscoveryBehaviourEvent {
    Identify(identify::Event),
}

impl From<identify::Event> for DiscoveryBehaviourEvent {
    fn from(event: identify::Event) -> Self {
        DiscoveryBehaviourEvent::Identify(event)
    }
}

#[derive(Debug)]
pub enum DiscoveryResult {
    DialableAddress(Multiaddr),
    NoDialableAddress,
}

pub async fn get_bootstrap_peers(
    addrbook: &AddrBook,
    discovery_config: &DiscoveryConfig,
) -> Result<Vec<(PeerId, Multiaddr)>> {
    let mut bootstrap_peers = Vec::new();

    // try addrbook first - get all dialable peers
    let dialable_peers = addrbook.get_dialable_peers();
    for (peer_id, multiaddr) in dialable_peers {
        event!(Level::INFO, %peer_id, %multiaddr, "using addrbook peer for discovery");
        bootstrap_peers.push((peer_id, multiaddr));
    }

    // fallback to DNS bootstrap if enabled and no addrbook peers
    if bootstrap_peers.is_empty() && discovery_config.dns.enabled {
        if let Some(enr) =
            get_dns_bootstrap_enr(&discovery_config.dns.bootstrap_domain)
                .await?
        {
            let multiaddrs = crate::peer_id::enr_to_multiaddrs(&enr);
            if !multiaddrs.is_empty() {
                let peer_id = crate::peer_id::enr_to_peer_id(&enr);
                // prefer TCP addresses for discovery connections
                let tcp_addrs: Vec<_> = multiaddrs
                    .iter()
                    .filter(|addr| {
                        addr.iter().any(|p| {
                            matches!(p, libp2p::multiaddr::Protocol::Tcp(_))
                        })
                    })
                    .collect();
                let multiaddr = if !tcp_addrs.is_empty() {
                    tcp_addrs[0].clone()
                } else {
                    multiaddrs[0].clone()
                };
                event!(Level::INFO, %peer_id, %multiaddr, "using DNS bootstrap peer for discovery");
                bootstrap_peers.push((peer_id, multiaddr));
            }
        }
    }

    Ok(bootstrap_peers)
}

pub async fn get_dns_bootstrap_enr(
    bootstrap_domain: &str,
) -> Result<Option<Enr>> {
    let resolver = TokioAsyncResolver::tokio_from_system_conf()?;

    let txt_records = resolver.txt_lookup(bootstrap_domain).await?;

    for record in txt_records.iter() {
        for txt_data in record.iter() {
            let txt_str = match std::str::from_utf8(txt_data) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if !txt_str.starts_with("enr:") {
                continue;
            }
            if let Ok(enr) = txt_str.parse::<Enr>() {
                return Ok(Some(enr));
            }
        }
    }

    Ok(None)
}

pub fn extract_ip_from_multiaddr(addr: &Multiaddr) -> Option<std::net::IpAddr> {
    for protocol in addr.iter() {
        match protocol {
            Protocol::Ip4(ip) => return Some(std::net::IpAddr::V4(ip)),
            Protocol::Ip6(ip) => return Some(std::net::IpAddr::V6(ip)),
            _ => continue,
        }
    }
    None
}

pub fn extract_port_from_multiaddr(addr: &Multiaddr) -> Option<u16> {
    use libp2p::multiaddr::Protocol;

    for protocol in addr.iter() {
        match protocol {
            Protocol::Tcp(port) => return Some(port),
            Protocol::Udp(port) => return Some(port),
            _ => continue,
        }
    }
    None
}

fn construct_multiaddr_from_ip_port(
    ip: std::net::IpAddr,
    port: u16,
) -> Multiaddr {
    let mut multiaddr = Multiaddr::empty();

    match ip {
        std::net::IpAddr::V4(ipv4) => {
            multiaddr.push(libp2p::multiaddr::Protocol::Ip4(ipv4));
        }
        std::net::IpAddr::V6(ipv6) => {
            multiaddr.push(libp2p::multiaddr::Protocol::Ip6(ipv6));
        }
    }

    multiaddr.push(libp2p::multiaddr::Protocol::Tcp(port));
    multiaddr
}

fn ip_protocol_matches(ip: std::net::IpAddr, listen_addr: &Multiaddr) -> bool {
    use libp2p::multiaddr::Protocol;

    for protocol in listen_addr.iter() {
        match (ip, protocol) {
            (std::net::IpAddr::V4(_), Protocol::Ip4(_)) => return true,
            (std::net::IpAddr::V6(_), Protocol::Ip6(_)) => return true,
            _ => continue,
        }
    }
    false
}

// construct startup ENR with no IP addresses - every node starts this way
pub fn build_startup_enr(discv5_key: &discv5::enr::CombinedKey) -> Result<Enr> {
    let mut enr_builder = discv5::enr::Enr::builder();
    let enr = enr_builder.build(discv5_key)?;
    Ok(enr)
}
