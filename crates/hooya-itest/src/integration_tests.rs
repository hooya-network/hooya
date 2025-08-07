use crate::init_tracing;
use crate::k3d::{check_k3d_available, check_kubectl_available};
use anyhow::Result;
use tracing::info;

/// Test that verifies k3d and kubectl tools are available
/// This is kept as a unit test to verify prerequisites
#[tokio::test]
#[ignore] // Requires k3d/kubectl to be installed
async fn test_k3d_tools_available() -> Result<()> {
    init_tracing();

    info!("Testing k3d and kubectl availability");

    check_k3d_available()?;
    check_kubectl_available()?;

    info!("✅ k3d and kubectl are available and working");

    Ok(())
}
