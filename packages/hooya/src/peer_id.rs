use discv5::Enr;
use libp2p::{multihash::Multihash, Multiaddr, PeerId};

/// Convert ENR to libp2p PeerId with 1:1 mapping from node_id
///
/// Since PeerId is just a wrapper around Multihash and NodeId is already
/// a 32-byte identifier, we create a direct deterministic mapping.
pub fn enr_to_peer_id(enr: &Enr) -> PeerId {
    let node_id = enr.node_id();

    // Create multihash directly from node_id raw bytes
    // Use Identity hash (0x00) since we already have a 32-byte hash
    let multihash = Multihash::wrap(0x00, &node_id.raw()).expect(
        "NodeId is exactly 32 bytes, Identity multihash should never fail",
    );

    PeerId::from_multihash(multihash)
        .expect("Valid multihash should always create valid PeerId")
}

// convert ENR to TCP multiaddrs for libp2p dialing
pub fn enr_to_tcp_multiaddrs(enr: &Enr) -> Vec<Multiaddr> {
    let mut multiaddrs = Vec::new();

    // get available IP addresses
    let ips = [enr.ip4().map(Into::into), enr.ip6().map(Into::into)]
        .into_iter()
        .flatten()
        .collect::<Vec<std::net::IpAddr>>();

    if ips.is_empty() {
        return multiaddrs;
    }

    // handle TCP ports (tcp4, tcp6)
    let tcp4_port = enr.tcp4();
    let tcp6_port = enr.tcp6();

    for ip in &ips {
        let port = match ip {
            std::net::IpAddr::V4(_) => tcp4_port,
            std::net::IpAddr::V6(_) => tcp6_port,
        };

        if let Some(port) = port {
            let mut multiaddr = Multiaddr::empty();
            match ip {
                std::net::IpAddr::V4(ipv4) => {
                    multiaddr.push(libp2p::multiaddr::Protocol::Ip4(*ipv4));
                }
                std::net::IpAddr::V6(ipv6) => {
                    multiaddr.push(libp2p::multiaddr::Protocol::Ip6(*ipv6));
                }
            }
            multiaddr.push(libp2p::multiaddr::Protocol::Tcp(port));
            multiaddrs.push(multiaddr);
        }
    }

    multiaddrs
}

// convert ENR to all multiaddrs (TCP and UDP) for comprehensive addressing
pub fn enr_to_multiaddrs(enr: &Enr) -> Vec<Multiaddr> {
    let mut multiaddrs = Vec::new();

    // get available IP addresses using iterator chain
    let ips = [enr.ip4().map(Into::into), enr.ip6().map(Into::into)]
        .into_iter()
        .flatten()
        .collect::<Vec<std::net::IpAddr>>();

    if ips.is_empty() {
        return multiaddrs;
    }

    // handle TCP ports (tcp4, tcp6)
    let tcp4_port = enr.tcp4();
    let tcp6_port = enr.tcp6();

    for ip in &ips {
        let port = match ip {
            std::net::IpAddr::V4(_) => tcp4_port,
            std::net::IpAddr::V6(_) => tcp6_port,
        };

        if let Some(port) = port {
            let mut multiaddr = Multiaddr::empty();
            match ip {
                std::net::IpAddr::V4(ipv4) => {
                    multiaddr.push(libp2p::multiaddr::Protocol::Ip4(*ipv4));
                }
                std::net::IpAddr::V6(ipv6) => {
                    multiaddr.push(libp2p::multiaddr::Protocol::Ip6(*ipv6));
                }
            }
            multiaddr.push(libp2p::multiaddr::Protocol::Tcp(port));
            multiaddrs.push(multiaddr);
        }
    }

    // handle UDP ports (udp4, udp6)
    let udp4_port = enr.udp4();
    let udp6_port = enr.udp6();

    for ip in &ips {
        let port = match ip {
            std::net::IpAddr::V4(_) => udp4_port,
            std::net::IpAddr::V6(_) => udp6_port,
        };

        if let Some(port) = port {
            let mut multiaddr = Multiaddr::empty();
            match ip {
                std::net::IpAddr::V4(ipv4) => {
                    multiaddr.push(libp2p::multiaddr::Protocol::Ip4(*ipv4));
                }
                std::net::IpAddr::V6(ipv6) => {
                    multiaddr.push(libp2p::multiaddr::Protocol::Ip6(*ipv6));
                }
            }
            multiaddr.push(libp2p::multiaddr::Protocol::Udp(port));
            multiaddrs.push(multiaddr);
        }
    }

    multiaddrs
}
