use crate::cluster::{ClusterTopology, NodeConfig};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct TestClaims {
    pub user_id: u64,
    pub exp: usize,
    pub iat: usize,
}

use crate::cluster::{create_test_cluster, DeployedCluster};
use std::sync::Arc;
use tokio::sync::OnceCell;

use crate::k3d::K3dCluster;
use tracing::info;

static CLUSTER_CELL: OnceCell<Arc<DeployedCluster>> = OnceCell::const_new();

/// Create and cache a deployed test cluster inside a running k3d instance
pub async fn get_shared_test_cluster() -> Arc<DeployedCluster> {
    CLUSTER_CELL
        .get_or_init(|| async {
            // 1. Spin up the k3d cluster (if not exists)
            let k3d = K3dCluster::new("hooya-itest".to_string());
            info!("Bringing up k3d cluster...");
            k3d.create().await.expect("Failed to create k3d cluster");
            k3d.wait_for_ready(120).await.expect("Cluster not ready");

            // 2. Deploy services (hooya & proxy)
            let topology = small_test_cluster();
            info!(
                "Deploying test topology: {} nodes, {} proxies",
                topology.nodes.len(),
                topology.proxies
            );
            let cluster = create_test_cluster(topology)
                .await
                .expect("Failed to deploy test cluster");

            port_forward_all_proxies(&cluster)
                .await
                .expect("port-forward failed");

            Arc::new(cluster)
        })
        .await
        .clone()
}

use anyhow::{Context, Result};
use tokio::process::Command;

/// Port-forward all proxy services to localhost for external access in tests.
async fn port_forward_all_proxies(cluster: &DeployedCluster) -> Result<()> {
    for i in 0..cluster.topology.proxies {
        let namespace = cluster.namespace.clone();
        let svc_name = format!("hooya-web-proxy-{}-service", i);
        let local_port = 8532 + i;

        tokio::spawn(async move {
            let output = Command::new("kubectl")
                .args([
                    "-n",
                    &namespace,
                    "port-forward",
                    &format!("service/{}", svc_name),
                    &format!("{}:8532", local_port.to_string()),
                ])
                .spawn()
                .context("Failed to start port-forward")
                .unwrap()
                .wait_with_output()
                .await;

            if let Err(e) = output {
                tracing::warn!("Port-forward error for proxy {}: {:?}", i, e);
            }
        });

        // Give it a bit of time to start
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Ok(())
}

/// Generate a fake JWT token for testing
pub fn generate_test_jwt() -> anyhow::Result<String> {
    let now = chrono::Utc::now().timestamp() as usize;
    let claims = TestClaims {
        user_id: 1,
        exp: now + 3600, // 1 hour
        iat: now,
    };

    // Use a simple test secret
    let secret = b"test-secret-key-32-bytes-long!!!";
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .map_err(|e| anyhow::anyhow!("Failed to generate test JWT: {}", e))?;

    Ok(token)
}

/// Create a test cluster topology with the specified number of nodes and proxies
pub fn create_test_topology(
    node_count: usize,
    proxy_count: u32,
    namespace_suffix: Option<&str>,
) -> ClusterTopology {
    let mut nodes = Vec::new();

    for i in 0..node_count {
        nodes.push(NodeConfig {
            name: format!("node-{}", i),
            operator: format!("test-operator-{}", i),
            instance_name: format!("test-instance-{}", i),
        });
    }

    let namespace = if let Some(suffix) = namespace_suffix {
        format!("hooya-itest-{}", suffix)
    } else {
        {
            let uuid_str = Uuid::new_v4().to_string();
            format!("hooya-itest-{}", uuid_str[..8].to_lowercase())
        }
    };

    ClusterTopology {
        nodes,
        proxies: proxy_count,
        namespace,
    }
}

/// Create a minimal test cluster (1 node, 1 proxy)
pub fn minimal_test_cluster() -> ClusterTopology {
    create_test_topology(1, 1, Some("minimal"))
}

/// Create a small test cluster (3 nodes, 2 proxies)
pub fn small_test_cluster() -> ClusterTopology {
    create_test_topology(3, 2, Some("small"))
}

/// Create a test cluster suitable for presence testing (5 nodes, 3 proxies)
pub fn presence_test_cluster() -> ClusterTopology {
    create_test_topology(5, 3, Some("presence"))
}

/// Test messages for chat scenarios
pub struct TestMessages;

impl TestMessages {
    pub const HELLO_WORLD: &'static str = "Hello, world!";
    pub const TEST_MESSAGE_1: &'static str = "This is test message 1";
    pub const TEST_MESSAGE_2: &'static str = "This is test message 2";
    pub const EMOJI_MESSAGE: &'static str = "Hello 👋 from test!";
    pub const LONG_MESSAGE: &'static str = "This is a longer test message that contains multiple words and should test message propagation across the mesh network with a reasonable amount of content to ensure everything works correctly.";

    /// Generate a unique test message
    pub fn unique_message() -> String {
        format!("Test message {}", Uuid::new_v4())
    }

    /// Generate a message with a specific prefix for filtering
    pub fn message_with_prefix(prefix: &str) -> String {
        format!("{}: {}", prefix, Uuid::new_v4())
    }
}

/// Test channels for chat scenarios
pub struct TestChannels;

impl TestChannels {
    pub const GENERAL: &'static str = "general";
    pub const RANDOM: &'static str = "random";
    pub const TEST: &'static str = "test";
    pub const INTEGRATION: &'static str = "integration";

    /// Generate a unique test channel name
    pub fn unique_channel() -> String {
        let uuid_str = Uuid::new_v4().to_string();
        format!("test-{}", &uuid_str[..8])
    }
}

/// Common test passwords
pub struct TestPasswords;

impl TestPasswords {
    pub const DEFAULT: &'static str = "test-password-123";
    pub const ADMIN: &'static str = "admin-password-456";
}

/// Helper to create a test cluster with custom node names
pub fn create_named_test_cluster(
    node_names: Vec<(&str, &str, &str)>, // (name, operator, instance)
    proxy_count: u32,
) -> ClusterTopology {
    let nodes = node_names
        .into_iter()
        .map(|(name, operator, instance)| NodeConfig {
            name: name.to_string(),
            operator: operator.to_string(),
            instance_name: instance.to_string(),
        })
        .collect();

    ClusterTopology {
        nodes,
        proxies: proxy_count,
        namespace: {
            let uuid_str = Uuid::new_v4().to_string();
            format!("hooya-itest-{}", uuid_str[..8].to_lowercase())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_test_jwt() {
        let token = generate_test_jwt().unwrap();
        assert!(!token.is_empty());
        assert!(token.contains('.')); // JWT should have dots
    }

    #[test]
    fn test_create_test_topology() {
        let topology = create_test_topology(3, 2, Some("test"));
        assert_eq!(topology.nodes.len(), 3);
        assert_eq!(topology.proxies, 2);
        assert!(topology.namespace.contains("test"));

        for (i, node) in topology.nodes.iter().enumerate() {
            assert_eq!(node.name, format!("node-{}", i));
            assert_eq!(node.operator, format!("test-operator-{}", i));
            assert_eq!(node.instance_name, format!("test-instance-{}", i));
        }
    }

    #[test]
    fn test_minimal_test_cluster() {
        let topology = minimal_test_cluster();
        assert_eq!(topology.nodes.len(), 1);
        assert_eq!(topology.proxies, 1);
    }

    #[test]
    fn test_small_test_cluster() {
        let topology = small_test_cluster();
        assert_eq!(topology.nodes.len(), 3);
        assert_eq!(topology.proxies, 2);
    }

    #[test]
    fn test_unique_message() {
        let msg1 = TestMessages::unique_message();
        let msg2 = TestMessages::unique_message();
        assert_ne!(msg1, msg2);
        assert!(msg1.starts_with("Test message"));
    }

    #[test]
    fn test_unique_channel() {
        let ch1 = TestChannels::unique_channel();
        let ch2 = TestChannels::unique_channel();
        assert_ne!(ch1, ch2);
        assert!(ch1.starts_with("test-"));
    }

    #[test]
    fn test_create_named_test_cluster() {
        let node_configs = vec![
            ("alice", "alice-operator", "alice-instance"),
            ("bob", "bob-operator", "bob-instance"),
        ];

        let topology = create_named_test_cluster(node_configs, 1);
        assert_eq!(topology.nodes.len(), 2);
        assert_eq!(topology.proxies, 1);

        assert_eq!(topology.nodes[0].name, "alice");
        assert_eq!(topology.nodes[0].operator, "alice-operator");
        assert_eq!(topology.nodes[0].instance_name, "alice-instance");

        assert_eq!(topology.nodes[1].name, "bob");
        assert_eq!(topology.nodes[1].operator, "bob-operator");
        assert_eq!(topology.nodes[1].instance_name, "bob-instance");
    }
}
