pub mod cluster;
pub mod expectations;
pub mod fixtures;
pub mod k3d;
pub mod rest;
pub mod sse;

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;

use std::time::Duration;

pub const DEFAULT_TEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_CLUSTER_NAMESPACE: &str = "hooya-itest";

#[derive(Debug, Clone)]
pub struct TestConfig {
    pub namespace: String,
    pub timeout: Duration,
    pub cleanup_on_failure: bool,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            namespace: DEFAULT_CLUSTER_NAMESPACE.to_string(),
            timeout: DEFAULT_TEST_TIMEOUT,
            cleanup_on_failure: true,
        }
    }
}

/// Initialize tracing for tests
pub fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("hooya_itest=debug".parse().unwrap()),
        )
        .try_init()
        .ok();
}
