use anyhow::Result;
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};
use tracing::{event, Level};

/// represents a peer we've discovered and can redial
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub enr: discv5::Enr, // ENR is now required - contains all addressing info
    pub last_seen: u64,   // unix timestamp
    pub connection_attempts: u32,
    pub last_connection_attempt: Option<u64>,
}

/// serializable version for toml storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPeer {
    enr: String, // store ENR as string for serialization - no multiaddr needed
}

/// manages persistent storage of known peers
pub struct AddrBook {
    peers: HashMap<PeerId, PeerInfo>,
    store_path: std::path::PathBuf,
    flush_interval: Duration,
    last_flush: Option<Instant>,
}

impl AddrBook {
    pub fn new(store_path: impl AsRef<Path>, flush_interval: Duration) -> Self {
        Self {
            peers: HashMap::new(),
            store_path: store_path.as_ref().to_path_buf(),
            flush_interval,
            last_flush: None,
        }
    }

    /// load peers from disk on startup
    pub async fn load(&mut self) -> Result<()> {
        if !self.store_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&self.store_path)?;
        let stored_peers: HashMap<String, StoredPeer> =
            toml::from_str(&content)?;

        let mut loaded_count = 0;
        let mut invalid_count = 0;

        // convert string keys back to PeerIds and validate ENRs
        for (peer_id_str, stored_peer) in stored_peers {
            if let Ok(peer_id) = peer_id_str.parse::<PeerId>() {
                // validate ENR is still parseable
                if let Ok(enr) = stored_peer.enr.parse::<discv5::Enr>() {
                    let now = chrono::Utc::now().timestamp() as u64;
                    self.peers.insert(
                        peer_id,
                        PeerInfo {
                            enr,
                            last_seen: now,
                            connection_attempts: 0,
                            last_connection_attempt: None,
                        },
                    );
                    loaded_count += 1;
                } else {
                    invalid_count += 1;
                }
            } else {
                invalid_count += 1;
            }
        }

        event!(Level::INFO,
            loaded_peers = loaded_count,
            invalid_peers = invalid_count,
            path = %self.store_path.display(),
            "loaded addr_book"
        );

        Ok(())
    }

    /// add or update a peer with ENR in the store
    pub async fn add_peer_with_enr(
        &mut self,
        peer_id: PeerId,
        enr: discv5::Enr,
    ) {
        let now = chrono::Utc::now().timestamp() as u64;

        self.peers
            .entry(peer_id)
            .and_modify(|info| {
                info.enr = enr.clone();
                info.last_seen = now;
            })
            .or_insert(PeerInfo {
                enr,
                last_seen: now,
                connection_attempts: 0,
                last_connection_attempt: None,
            });

        if let Err(e) = self.maybe_flush().await {
            event!(Level::ERROR, %e, "failed to flush addr_book after adding peer");
        }
    }

    /// remove a peer from the store
    pub async fn remove_peer(&mut self, peer_id: &PeerId) {
        self.peers.remove(peer_id);

        if let Err(e) = self.maybe_flush().await {
            event!(Level::ERROR, %e, "failed to flush addr_book after removing peer");
        }
    }

    /// get all known peers
    pub fn get_peers(&self) -> &HashMap<PeerId, PeerInfo> {
        &self.peers
    }

    /// record a connection attempt for a peer
    pub fn record_connection_attempt(
        &mut self,
        peer_id: &PeerId,
        successful: bool,
    ) {
        if let Some(info) = self.peers.get_mut(peer_id) {
            info.connection_attempts += 1;
            info.last_connection_attempt =
                Some(chrono::Utc::now().timestamp() as u64);

            // remove peer if too many failed attempts
            if !successful && info.connection_attempts > 5 {
                self.peers.remove(peer_id);
            }
        }
    }

    /// check if it's time to flush and do so if needed
    pub async fn maybe_flush(&mut self) -> Result<()> {
        let now = Instant::now();

        let should_flush = match self.last_flush {
            Some(last) => now.duration_since(last) >= self.flush_interval,
            None => true, // flush immediately if never flushed
        } && !self.peers.is_empty();

        if should_flush {
            let peer_count = self.peers.len();
            event!(Level::INFO,
                peer_count = peer_count,
                path = %self.store_path.display(),
                "flushing addr_book to disk"
            );
            self.flush().await?;
            self.last_flush = Some(now);
        }

        Ok(())
    }

    /// force flush peers to disk
    pub async fn flush(&self) -> Result<()> {
        // ensure parent directory exists
        if let Some(parent) = self.store_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let serializable_peers: HashMap<String, StoredPeer> = self
            .peers
            .iter()
            .map(|(peer_id, info)| {
                (
                    peer_id.to_string(),
                    StoredPeer {
                        enr: info.enr.to_string(),
                    },
                )
            })
            .collect();

        let content = toml::to_string_pretty(&serializable_peers)?;
        tokio::fs::write(&self.store_path, content).await?;

        Ok(())
    }

    /// get peers that should be re-dialed on startup
    /// excludes peers that have failed too many times recently
    pub fn get_dialable_peers(&self) -> Vec<(PeerId, Multiaddr)> {
        let now = chrono::Utc::now().timestamp() as u64;
        let hour_ago = now - 3600; // 1 hour ago

        self.peers
            .iter()
            .filter(|(_, info)| {
                // only include peers that haven't failed recently
                info.connection_attempts < 3
                    || info
                        .last_connection_attempt
                        .is_none_or(|last| last < hour_ago)
            })
            .filter_map(|(peer_id, info)| {
                // derive multiaddr from ENR - use first available TCP address
                let multiaddrs =
                    crate::peer_id::enr_to_tcp_multiaddrs(&info.enr);
                multiaddrs
                    .first()
                    .map(|multiaddr| (*peer_id, multiaddr.clone()))
            })
            .collect()
    }

    /// check if we know this peer
    pub fn contains_peer(&self, peer_id: &PeerId) -> bool {
        self.peers.contains_key(peer_id)
    }
}
