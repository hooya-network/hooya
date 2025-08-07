use crate::cluster::DeployedCluster;
use crate::expectations::{ChatExpectations, ExpectationBuilder, TestUtils};
use crate::fixtures::{TestChannels, TestMessages};
use crate::rest::RestClient;
use crate::sse::SseClient;
use crate::{init_tracing, TestConfig};
use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{info, warn};

use crate::fixtures::get_shared_test_cluster;

/// Integration test for chat message propagation across the mesh network
#[tokio::test]
async fn test_chat_message_propagation() {
    init_tracing();
    let config = TestConfig::default();

    let cluster = get_shared_test_cluster().await;

    let result = run_chat_propagation_test(&cluster, &config).await;

    if let Err(e) = &result {
        tracing::error!("Test failed: {:?}", e);
        if config.cleanup_on_failure {
            cluster.cleanup().await.unwrap();
        }
    } else {
        cluster.cleanup().await.unwrap();
    }

    assert!(result.is_ok(), "Chat propagation test failed: {:?}", result);
}

async fn run_chat_propagation_test(
    cluster: &DeployedCluster,
    config: &TestConfig,
) -> Result<()> {
    info!("Setting up REST and SSE clients");

    // Log extracted ENRs for debugging
    for (i, enr) in cluster.node_enrs.iter().enumerate() {
        match enr {
            Some(enr) => info!("Node {} ENR: {}", i, enr),
            None => warn!("Node {} ENR not found", i),
        }
    }

    // Set up port forwarding or use service addresses
    // For now, assume we have direct access to services within the cluster
    let proxy_urls: Vec<String> = (0..cluster.topology.proxies)
        .map(|i| format!("http://localhost:{}", 8532 + i))
        .collect();

    // Create REST clients for each proxy
    let mut rest_clients = Vec::new();
    for (i, url) in proxy_urls.iter().enumerate() {
        info!("Waiting for proxy {} to become healthy", i);
        let mut client = RestClient::new(url.clone());

        // Wait for proxy to be healthy first
        client
            .wait_for_health(Duration::from_secs(60))
            .await
            .map_err(|e| {
                anyhow::anyhow!("Proxy {} failed to become healthy: {}", i, e)
            })?;

        // Authenticate using the extracted password
        if let Some(password) = cluster.proxy_password(i) {
            info!("Authenticating with proxy {} using extracted password", i);
            client.login(password).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to authenticate with proxy {}: {}",
                    i,
                    e
                )
            })?;
        } else {
            return Err(anyhow::anyhow!(
                "No password found for proxy {}, cannot authenticate",
                i
            ));
        }

        rest_clients.push(client);
    }

    // Create SSE clients to listen for events
    let mut sse_clients = Vec::new();
    for (i, url) in proxy_urls.iter().enumerate() {
        let mut sse_client = SseClient::new(url.clone());

        // Use the same authentication token from the REST client
        if let Some(token) = rest_clients[i].auth_token() {
            sse_client = sse_client.with_auth(token.to_string());
        }

        sse_clients.push(sse_client);
    }

    info!("Connecting to SSE streams");
    let mut sse_streams = Vec::new();
    for client in &sse_clients {
        let stream = client.connect_instance_events().await?;
        sse_streams.push(stream);
    }

    // Send a test message through the first proxy
    let test_message = TestMessages::unique_message();
    let test_channel = TestChannels::GENERAL;

    info!(
        "Sending test message '{}' to channel '{}' via proxy 0",
        test_message, test_channel
    );

    let _response = rest_clients[0]
        .send_chat_message(test_channel, &test_message)
        .await?;

    info!("Message sent, collecting events from all SSE streams");

    // Collect events from all streams, with reduced timeout since messages should propagate quickly
    let collection_duration = Duration::from_secs(3); // Reduced from 10 seconds
    let mut all_events = Vec::new();

    let start_time = std::time::Instant::now();
    for (i, stream) in sse_streams.iter_mut().enumerate() {
        info!("Collecting events from stream {}", i);
        let events = stream.collect_events(collection_duration).await;
        let elapsed = start_time.elapsed();
        info!(
            "Stream {} received {} events in {:?}",
            i,
            events.len(),
            elapsed
        );
        TestUtils::log_event_summary(&events);
        all_events.extend(events);
    }

    info!("Total collection time: {:?}", start_time.elapsed());

    info!("Total events collected: {}", all_events.len());

    // Assert that we received the message
    let result = ExpectationBuilder::new()
        .expect_chat_from_node(
            &all_events,
            "", // We don't know the exact node_id yet, so this will fail
            test_channel,
            &test_message,
        )
        .build();

    // For now, just check if we got any chat events at all
    let chat_events = ChatExpectations::extract_chat_events(&all_events);
    if chat_events.is_empty() {
        return Err(anyhow::anyhow!(
            "No chat events received - this could indicate:\n\
             1. Authentication issues (no JWT token)\n\
             2. Network connectivity problems\n\
             3. SSE stream setup issues\n\
             4. Proxy-to-node connection problems"
        ));
    }

    info!("✓ Received {} chat events", chat_events.len());
    for (i, event) in chat_events.iter().enumerate() {
        info!(
            "  Chat event {}: {} in {} from {}",
            i + 1,
            event.content,
            event.channel,
            event.node_id
        );
    }

    // Check if any of the chat events match our test message
    // Note: Chat messages include timestamp and node ID prefix, so we need to check if our message is contained in the content
    let found_our_message = chat_events.iter().any(|event| {
        event.channel == test_channel && event.content.contains(&test_message)
    });

    if found_our_message {
        info!("✓ Found our test message in the chat events");
        Ok(())
    } else {
        info!("Expected message content: '{}'", test_message);
        info!("Received message contents:");
        for (i, event) in chat_events.iter().enumerate() {
            info!("  Event {}: '{}'", i + 1, event.content);
        }
        Err(anyhow::anyhow!(
            "Our test message '{}' was not found in {} chat events",
            test_message,
            chat_events.len()
        ))
    }
}

/// Test presence functionality (once implemented)
#[tokio::test]
#[ignore] // Ignored until presence is implemented
async fn test_presence_propagation() -> Result<()> {
    init_tracing();

    info!("🚀 Starting presence propagation test");

    // This test will be implemented once presence functionality is added
    // It should:
    // 1. Deploy a cluster with multiple nodes
    // 2. Simulate users joining channels via different proxies
    // 3. Verify that JOIN/LEAVE events propagate across the mesh
    // 4. Test heartbeat mechanisms

    todo!("Implement once presence protocol is added");
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::fixtures::minimal_test_cluster;

    #[test]
    fn test_test_config_defaults() {
        let config = TestConfig::default();
        assert_eq!(config.namespace, "hooya-itest");
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.cleanup_on_failure);
    }

    #[test]
    fn test_cluster_topology_creation() {
        let topology = minimal_test_cluster();
        assert_eq!(topology.nodes.len(), 1);
        assert_eq!(topology.proxies, 1);
        assert!(topology.namespace.starts_with("hooya-itest-"));
    }
}
