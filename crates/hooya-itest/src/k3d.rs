use anyhow::{Context, Result};
use std::process::Command;
use tracing::{debug, info, warn};

/// k3d cluster management for integration tests
pub struct K3dCluster {
    pub name: String,
    pub registry_port: u16,
}

impl K3dCluster {
    pub fn new(name: String) -> Self {
        Self {
            name,
            registry_port: 5000,
        }
    }

    /// Create a new k3d cluster for testing
    pub async fn create(&self) -> Result<()> {
        info!("Creating k3d cluster: {}", self.name);

        // Check if cluster already exists
        if self.exists().await? {
            info!("Cluster {} already exists, deleting first", self.name);
            self.delete().await?;
        }

        // Create local registry for faster image pulls
        self.create_registry().await?;

        // Create k3d cluster with registry
        let output = Command::new("k3d")
            .args([
                "cluster",
                "create",
                &self.name,
                "--registry-use",
                &format!("k3d-{}-registry:{}", self.name, self.registry_port),
                "--api-port",
                "6443",
                "--servers",
                "1",
                "--agents",
                "2", // Some agents for more realistic testing
                "--wait",
            ])
            .output()
            .context("Failed to execute k3d cluster create command")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "k3d cluster create failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!("Successfully created k3d cluster: {}", self.name);

        // Set kubectl context
        self.set_kubectl_context().await?;

        // Import local test images into the cluster
        self.import_test_images().await?;

        Ok(())
    }

    /// Delete the k3d cluster
    pub async fn delete(&self) -> Result<()> {
        if !self.exists().await? {
            debug!("Cluster {} does not exist, nothing to delete", self.name);
            return Ok(());
        }

        info!("Deleting k3d cluster: {}", self.name);

        let output = Command::new("k3d")
            .args(["cluster", "delete", &self.name])
            .output()
            .context("Failed to execute k3d cluster delete command")?;

        if !output.status.success() {
            warn!(
                "k3d cluster delete had issues: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Also delete the registry
        self.delete_registry().await?;

        info!("Deleted k3d cluster: {}", self.name);
        Ok(())
    }

    /// Check if the cluster exists
    pub async fn exists(&self) -> Result<bool> {
        let output = Command::new("k3d")
            .args(["cluster", "list", "--output", "json"])
            .output()
            .context("Failed to list k3d clusters")?;

        if !output.status.success() {
            return Ok(false);
        }

        let clusters_json = String::from_utf8_lossy(&output.stdout);
        Ok(clusters_json.contains(&format!("\"name\":\"k3d-{}\"", self.name)))
    }

    /// Set kubectl context to use this cluster
    async fn set_kubectl_context(&self) -> Result<()> {
        let context_name = format!("k3d-{}", self.name);

        let output = Command::new("kubectl")
            .args(["config", "use-context", &context_name])
            .output()
            .context("Failed to set kubectl context")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to set kubectl context: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!("Set kubectl context to: {}", context_name);
        Ok(())
    }

    /// Create a local docker registry for the cluster
    async fn create_registry(&self) -> Result<()> {
        let registry_name = format!("k3d-{}-registry", self.name);

        // Check if registry already exists
        let output = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--format",
                "{{.Names}}",
                "--filter",
                &format!("name={registry_name}"),
            ])
            .output()
            .context("Failed to check for existing registry")?;

        if output.status.success()
            && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        {
            debug!("Registry {} already exists", registry_name);
            return Ok(());
        }

        // Create registry
        let output = Command::new("k3d")
            .args([
                "registry",
                "create",
                &format!("{}-registry", self.name),
                "--port",
                &self.registry_port.to_string(),
            ])
            .output()
            .context("Failed to create k3d registry")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to create k3d registry: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!("Created registry: {}", registry_name);
        Ok(())
    }

    /// Delete the local docker registry
    async fn delete_registry(&self) -> Result<()> {
        let output = Command::new("k3d")
            .args(["registry", "delete", &format!("{}-registry", self.name)])
            .output()
            .context("Failed to delete k3d registry")?;

        if !output.status.success() {
            warn!(
                "k3d registry delete had issues: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Wait for cluster to be ready
    pub async fn wait_for_ready(&self, timeout_secs: u64) -> Result<()> {
        info!("Waiting for cluster {} to be ready...", self.name);

        for i in 0..timeout_secs {
            let output = Command::new("kubectl")
                .args(["get", "nodes", "--no-headers"])
                .output()
                .context("Failed to check cluster readiness")?;

            if output.status.success() {
                let nodes_output = String::from_utf8_lossy(&output.stdout);
                let ready_nodes = nodes_output
                    .lines()
                    .filter(|line| line.contains("Ready"))
                    .count();

                if ready_nodes >= 1 {
                    // At least 1 node should be ready
                    info!(
                        "Cluster {} is ready with {} nodes",
                        self.name, ready_nodes
                    );
                    return Ok(());
                }
            }

            if i % 10 == 0 {
                debug!(
                    "Still waiting for cluster readiness... ({}/{})",
                    i, timeout_secs
                );
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        Err(anyhow::anyhow!(
            "Cluster {} did not become ready within {} seconds",
            self.name,
            timeout_secs
        ))
    }

    /// Get the kubeconfig for this cluster
    pub fn get_kubeconfig(&self) -> Result<String> {
        let output = Command::new("k3d")
            .args(["kubeconfig", "get", &self.name])
            .output()
            .context("Failed to get kubeconfig")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to get kubeconfig: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Import local test images into the k3d cluster
    async fn import_test_images(&self) -> Result<()> {
        info!("Importing local test images into k3d cluster");

        // Import hooyad image
        let output = Command::new("k3d")
            .args(["image", "import", "hooyad:itests", "--cluster", &self.name])
            .output()
            .context("Failed to import hooyad image")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to import hooyad:itests image: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        // Import hooya-web-proxy image
        let output = Command::new("k3d")
            .args([
                "image",
                "import",
                "hooya-web-proxy:itests",
                "--cluster",
                &self.name,
            ])
            .output()
            .context("Failed to import hooya-web-proxy image")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Failed to import hooya-web-proxy:itests image: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        info!("Successfully imported test images into k3d cluster");
        Ok(())
    }
}

/// Check if k3d is available on the system
pub fn check_k3d_available() -> Result<()> {
    let output = Command::new("k3d")
        .args(["version"])
        .output()
        .context("k3d command not found - please install k3d first")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("k3d is not working properly"));
    }

    let version = String::from_utf8_lossy(&output.stdout);
    info!(
        "Found k3d: {}",
        version.lines().next().unwrap_or("unknown version")
    );
    Ok(())
}

/// Check if kubectl is available on the system  
pub fn check_kubectl_available() -> Result<()> {
    let output = Command::new("kubectl")
        .args(["version", "--client"])
        .output()
        .context("kubectl command not found - please install kubectl first")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!("kubectl is not working properly"));
    }

    let version = String::from_utf8_lossy(&output.stdout);
    info!("Found kubectl: {}", version.trim());
    Ok(())
}

// Removed manual lifecycle test to keep CI focused on e2e
