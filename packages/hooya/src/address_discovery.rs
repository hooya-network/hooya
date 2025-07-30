use crate::addr_book::AddrBook;
use anyhow::Result;
use discv5::Enr;
use hooya_config::{DiscoveryConfig, NetworkingConfig};
use libp2p::multiaddr::Protocol;
use libp2p::{
    identify::{self, Behaviour as IdentifyBehaviour},
    swarm::NetworkBehaviour,
    Multiaddr, PeerId,
};
use tokio::sync::oneshot;
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

pub struct AddressDiscovery {
    listen_addresses: Vec<Multiaddr>,
    result_sender: Option<oneshot::Sender<DiscoveryResult>>,
}

impl AddressDiscovery {
    pub fn new(
        listen_addresses: Vec<Multiaddr>,
        result_sender: oneshot::Sender<DiscoveryResult>,
    ) -> Self {
        Self {
            listen_addresses,
            result_sender: Some(result_sender),
        }
    }

    pub fn handle_identify_event(&mut self, event: &identify::Event) {
        if let identify::Event::Received { info, .. } = event {
            // extract IP from observed_addr
            let observed_ip = extract_ip_from_multiaddr(&info.observed_addr);

            if let Some(ip) = observed_ip {
                // constructs a probably dialable address by combining observed_addr
                // with the port we listen on. We can't use observed_addr directly
                // because the port is going to be different than the one we are
                // listening on.
                for listen_addr in &self.listen_addresses {
                    if ip_protocol_matches(ip, listen_addr) {
                        if let Some(port) =
                            extract_port_from_multiaddr(listen_addr)
                        {
                            let constructed_addr =
                                construct_multiaddr_from_ip_port(ip, port);

                            if let Some(sender) = self.result_sender.take() {
                                let _ = sender.send(
                                    DiscoveryResult::DialableAddress(
                                        constructed_addr,
                                    ),
                                );
                            }
                            return;
                        }
                    }
                }
            }

            // we were unable to find a dialable address. In the future we
            // should use autonat to determine if this is indeed dialable. For
            // now we just assume it is
            if let Some(sender) = self.result_sender.take() {
                let _ = sender.send(DiscoveryResult::NoDialableAddress);
            }
        }
    }
}

pub async fn get_bootstrap_peer(
    addrbook: &AddrBook,
    discovery_config: &DiscoveryConfig,
) -> Result<Option<(PeerId, Multiaddr)>> {
    // try addrbook first
    let dialable_peers = addrbook.get_dialable_peers();
    if !dialable_peers.is_empty() {
        let (peer_id, multiaddr) = &dialable_peers[0];
        event!(Level::INFO, %peer_id, %multiaddr, "using addrbook peer for discovery");
        return Ok(Some((*peer_id, multiaddr.clone())));
    }

    // fallback to DNS bootstrap if enabled and addrbook empty
    if discovery_config.dns.enabled {
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
                return Ok(Some((peer_id, multiaddr)));
            }
        }
    }

    Ok(None)
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

fn extract_ip_from_multiaddr(addr: &Multiaddr) -> Option<std::net::IpAddr> {
    for protocol in addr.iter() {
        match protocol {
            Protocol::Ip4(ip) => return Some(std::net::IpAddr::V4(ip)),
            Protocol::Ip6(ip) => return Some(std::net::IpAddr::V6(ip)),
            _ => continue,
        }
    }
    None
}

fn extract_port_from_multiaddr(addr: &Multiaddr) -> Option<u16> {
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

// construct ENR from advertise IPs or discovered dialable addresses
pub fn build_enr_from_addresses(
    ips: &[std::net::IpAddr],
    config: &NetworkingConfig,
    discv5_key: &discv5::enr::CombinedKey,
) -> Result<Enr> {
    if ips.is_empty() {
        return Err(anyhow::anyhow!("no dialable addresses"));
    }

    // extract TCP ports from listen_addresses
    let mut tcp4_port = None;
    let mut tcp6_port = None;
    for addr_str in &config.listen_addresses {
        if let Ok(addr) = addr_str.parse::<Multiaddr>() {
            let mut is_ipv4 = false;
            let mut is_ipv6 = false;
            let mut tcp_port = None;

            for protocol in addr.iter() {
                match protocol {
                    libp2p::multiaddr::Protocol::Ip4(_) => is_ipv4 = true,
                    libp2p::multiaddr::Protocol::Ip6(_) => is_ipv6 = true,
                    libp2p::multiaddr::Protocol::Tcp(port) => {
                        tcp_port = Some(port)
                    }
                    _ => {}
                }
            }

            if let Some(port) = tcp_port {
                if is_ipv4 && tcp4_port.is_none() {
                    tcp4_port = Some(port);
                }
                if is_ipv6 && tcp6_port.is_none() {
                    tcp6_port = Some(port);
                }
            }
        }
    }

    // extract UDP ports from discv5_listen_addresses
    let mut udp4_port = None;
    let mut udp6_port = None;
    for addr_str in &config.discv5_listen_addresses {
        if let Ok(addr) = addr_str.parse::<Multiaddr>() {
            let mut is_ipv4 = false;
            let mut is_ipv6 = false;
            let mut udp_port = None;

            for protocol in addr.iter() {
                match protocol {
                    libp2p::multiaddr::Protocol::Ip4(_) => is_ipv4 = true,
                    libp2p::multiaddr::Protocol::Ip6(_) => is_ipv6 = true,
                    libp2p::multiaddr::Protocol::Udp(port) => {
                        udp_port = Some(port)
                    }
                    _ => {}
                }
            }

            if let Some(port) = udp_port {
                if is_ipv4 && udp4_port.is_none() {
                    udp4_port = Some(port);
                }
                if is_ipv6 && udp6_port.is_none() {
                    udp6_port = Some(port);
                }
            }
        }
    }

    // build ENR with IP + TCP + UDP ports
    let mut enr_builder = discv5::enr::Enr::builder();
    let mut has_address = false;

    for ip in ips {
        match ip {
            std::net::IpAddr::V4(ipv4) => {
                if let (Some(tcp), Some(udp)) = (tcp4_port, udp4_port) {
                    enr_builder.ip4(*ipv4).tcp4(tcp).udp4(udp);
                    has_address = true;
                    break; // use first valid IPv4
                }
            }
            std::net::IpAddr::V6(ipv6) => {
                if let (Some(tcp), Some(udp)) = (tcp6_port, udp6_port) {
                    enr_builder.ip6(*ipv6).tcp6(tcp).udp6(udp);
                    has_address = true;
                    break; // use first valid IPv6
                }
            }
        }
    }

    if !has_address {
        return Err(anyhow::anyhow!(
            "no valid IP:port combinations - missing TCP or UDP ports for advertised IPs"
        ));
    }

    let enr = enr_builder.build(discv5_key)?;
    Ok(enr)
}

/// run address discovery phase to determine our dialable address
pub async fn run_discovery_phase(
    networking_config: &NetworkingConfig,
    libp2p_key: libp2p::identity::Keypair,
    bootstrap_peer: (PeerId, Multiaddr),
) -> Result<Vec<Multiaddr>> {
    use futures_util::StreamExt;
    use libp2p::{identify, SwarmBuilder};

    // parse listen addresses - inline simple operation
    let listen_addrs: Vec<Multiaddr> = networking_config
        .listen_addresses
        .iter()
        .map(|addr| addr.parse())
        .collect::<Result<Vec<_>, _>>()?;

    // create result channel - inline
    let (result_sender, result_receiver) = oneshot::channel();
    let mut discovery = AddressDiscovery::new(listen_addrs, result_sender);

    // create discovery swarm with inline behavior setup
    let behavior = DiscoveryBehaviour {
        identify: identify::Behaviour::new(identify::Config::new(
            "/hooya/discovery/1.0.0".to_string(),
            libp2p_key.public(),
        )),
    };

    let mut swarm = SwarmBuilder::with_existing_identity(libp2p_key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_behaviour(|_| behavior)?
        .build();

    // setup listeners
    for addr_str in &networking_config.listen_addresses {
        let listen_addr: Multiaddr = addr_str.parse()?;
        swarm.listen_on(listen_addr)?;
    }

    // connect to bootstrap peer
    let (_bootstrap_peer_id, bootstrap_addr) = &bootstrap_peer;
    swarm.dial(bootstrap_addr.clone())?;

    // discovery event loop
    tokio::select! {
        result = result_receiver => {
            match result.map_err(|_| anyhow::anyhow!("discovery channel closed"))? {
                DiscoveryResult::DialableAddress(addr) => Ok(vec![addr]),
                DiscoveryResult::NoDialableAddress => Err(anyhow::anyhow!("no dialable addresses")),
            }
        }
        _ = async {
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => {
                        match event {
                            libp2p::swarm::SwarmEvent::Behaviour(DiscoveryBehaviourEvent::Identify(identify_event)) => {
                                discovery.handle_identify_event(&identify_event);
                            }
                            libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                tracing::info!(%peer_id, "discovery connection established");
                            }
                            libp2p::swarm::SwarmEvent::OutgoingConnectionError { error, .. } => {
                                tracing::error!(%error, "could not dial peer");
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
        } => {
            Err(anyhow::anyhow!("unable to discover our dialable address"))
        }
    }
}
