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
use tracing::{debug, info};

static CLUSTER_CELL: OnceCell<Arc<DeployedCluster>> = OnceCell::const_new();
static POSTGRES_CELL: OnceCell<Arc<PostgresContainer>> = OnceCell::const_new();

/// Create and cache a deployed test cluster inside a running k3d instance
pub async fn get_shared_test_cluster() -> Arc<DeployedCluster> {
    CLUSTER_CELL
        .get_or_init(|| async {
            // 1. Spin up the k3d cluster (if not exists)
            let k3d = K3dCluster::new("hooya-itest".to_string());
            info!("Bringing up k3d cluster...");
            k3d.create().await.expect("Failed to create k3d cluster");
            k3d.wait_for_ready(120).await.expect("Cluster not ready");

            // 2. Ensure PostgreSQL is available and build topology (one node uses Postgres)
            let pg = get_shared_postgres_container().await;
            let pg_uri = pg
                .k8s_connection_string()
                .await
                .expect("Failed to get k8s connection string");

            let mut topology = small_test_cluster();
            if let Some(first) = topology.nodes.get_mut(0) {
                first.db_uri = Some(pg_uri);
                info!(
                    "Configured node '{}' to use Postgres backend",
                    first.name
                );
            }
            // configure remaining nodes to use sqlite in the mounted /data directory
            for (i, node) in topology.nodes.iter_mut().enumerate().skip(1) {
                node.db_uri = Some("sqlite:///data/hooya.sqlite".to_string());
                info!(
                    "Configured node '{}' to use SQLite backend at /data",
                    node.name
                );
            }

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

/// Create and cache a shared PostgreSQL container for all tests
pub async fn get_shared_postgres_container() -> Arc<PostgresContainer> {
    POSTGRES_CELL
        .get_or_init(|| async {
            info!("Starting shared PostgreSQL container...");
            Arc::new(
                PostgresContainer::start()
                    .await
                    .expect("Failed to start shared postgres container"),
            )
        })
        .await
        .clone()
}

use anyhow::{Context, Result};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::sleep;

/// Minimal Postgres container manager for tests
pub struct PostgresContainer {
    container_id: String,
    port: u16,
}

impl PostgresContainer {
    pub async fn start() -> Result<Self> {
        let port = 15432; // fixed host port for tests
        let container_name =
            format!("hooya-test-postgres-{}", uuid::Uuid::new_v4().simple());

        info!("Starting PostgreSQL container: {}", container_name);

        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "--name",
                &container_name,
                "-e",
                "POSTGRES_PASSWORD=testpass",
                "-e",
                "POSTGRES_USER=testuser",
                "-e",
                "POSTGRES_DB=hooya_test",
                "-p",
                &format!("{port}:5432"),
                "postgres:15",
            ])
            .output()
            .await
            .context("Failed to start postgres container")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Docker run failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let container_id = String::from_utf8(output.stdout)?.trim().to_string();
        info!("PostgreSQL container started: {}", container_id);

        // Connect to k3d network for inter-container communication
        let connect_output = Command::new("docker")
            .args(["network", "connect", "k3d-hooya-itest", &container_id])
            .output()
            .await
            .context("Failed to connect postgres to k3d network")?;

        if !connect_output.status.success() {
            info!(
                "Failed to connect to k3d network (may not exist yet): {}",
                String::from_utf8_lossy(&connect_output.stderr)
            );
        } else {
            info!("Connected PostgreSQL container to k3d network");
        }

        // Wait for readiness
        let mut retries = 30;
        while retries > 0 {
            let check_output = Command::new("docker")
                .args([
                    "exec",
                    &container_id,
                    "pg_isready",
                    "-U",
                    "testuser",
                    "-d",
                    "hooya_test",
                ])
                .output()
                .await?;
            if check_output.status.success() {
                info!("PostgreSQL is ready");
                break;
            }
            debug!(
                "Waiting for PostgreSQL to be ready... {} retries left",
                retries
            );
            sleep(std::time::Duration::from_secs(1)).await;
            retries -= 1;
        }
        if retries == 0 {
            return Err(anyhow::anyhow!(
                "PostgreSQL container failed to become ready"
            ));
        }

        Ok(Self { container_id, port })
    }

    /// Get the container's IP address on the k3d network
    pub async fn get_k3d_ip(&self) -> Result<Option<String>> {
        let output = Command::new("docker")
            .args(["inspect", &self.container_id, "--format", "{{(index .NetworkSettings.Networks \"k3d-hooya-itest\").IPAddress}}"])
            .output()
            .await
            .context("Failed to get container IP")?;

        if output.status.success() {
            let ip = String::from_utf8(output.stdout)?.trim().to_string();
            if !ip.is_empty() && ip != "<nil>" {
                return Ok(Some(ip));
            }
        }
        Ok(None)
    }

    pub fn connection_string(&self) -> String {
        format!(
            "postgresql://testuser:testpass@localhost:{}/hooya_test",
            self.port
        )
    }

    pub async fn k8s_connection_string(&self) -> Result<String> {
        // Try to get IP from k3d network first
        if let Some(k3d_ip) = self.get_k3d_ip().await? {
            info!("Using PostgreSQL k3d network IP: {}", k3d_ip);
            return Ok(format!(
                "postgresql://testuser:testpass@{k3d_ip}:5432/hooya_test"
            ));
        }

        // Fallback to host access (original behavior)
        info!("Falling back to host.k3d.internal access");
        Ok(format!(
            "postgresql://testuser:testpass@host.k3d.internal:{}/hooya_test",
            self.port
        ))
    }

    pub async fn cleanup(&self) -> Result<()> {
        let _ = Command::new("docker")
            .args(["stop", &self.container_id])
            .output()
            .await;
        let _ = Command::new("docker")
            .args(["rm", &self.container_id])
            .output()
            .await;
        Ok(())
    }
}

/// Port-forward all proxy services to localhost for external access in tests.
pub async fn port_forward_all_proxies(cluster: &DeployedCluster) -> Result<()> {
    // Ensure kubectl is available
    let kubectl_ok = Command::new("kubectl")
        .arg("version")
        .arg("--client")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !kubectl_ok {
        return Err(anyhow::anyhow!("kubectl is not installed or not working (required for port-forward)"));
    }

    for i in 0..cluster.topology.proxies {
        let namespace = cluster.namespace.clone();
        let local_port = 8532 + i;
        let service_name = format!("hooya-web-proxy-{i}-service");
        let service_name_for_task = service_name.clone();

        // Spawn a resilient port-forward loop to the Service that restarts on disconnect
        tokio::spawn(async move {
            loop {
                let mut child = Command::new("kubectl")
                    .args([
                        "-n",
                        &namespace,
                        "port-forward",
                        &format!("service/{service_name_for_task}"),
                        &format!("{local_port}:8532"),
                    ])
                    .spawn()
                    .expect("Failed to start port-forward process");
                let status = child.wait().await;
                tracing::warn!(
                    "Port-forward exited for proxy service {} ({}): {:?}",
                    i,
                    service_name_for_task,
                    status
                );
                // Small backoff before retrying
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        // Wait until the local port is accepting connections (up to ~10s)
        let mut ready = false;
        for _ in 0..50 {
            match TcpStream::connect(("127.0.0.1", local_port as u16)).await {
                Ok(_) => {
                    ready = true;
                    break;
                }
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(200))
                        .await;
                }
            }
        }
        if !ready {
            tracing::warn!(
                "Port-forward to service {} did not become ready on localhost:{}",
                service_name,
                local_port
            );
        } else {
            tracing::info!(
                "Established port-forward to service {} on localhost:{}",
                service_name,
                local_port
            );
        }
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
            name: format!("node-{i}"),
            operator: format!("test-operator-{i}"),
            instance_name: format!("test-instance-{i}"),
            db_uri: None,
        });
    }

    let namespace = if let Some(suffix) = namespace_suffix {
        format!("hooya-itest-{suffix}")
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
            db_uri: None,
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
            assert_eq!(node.name, format!("node-{i}"));
            assert_eq!(node.operator, format!("test-operator-{i}"));
            assert_eq!(node.instance_name, format!("test-instance-{i}"));
            assert_eq!(node.db_uri, None);
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
