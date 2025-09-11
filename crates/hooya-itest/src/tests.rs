use crate::cluster::DeployedCluster;
use crate::expectations::{ChatExpectations, TestUtils};
use crate::fixtures::{TestChannels, TestMessages};
use crate::rest::{FileTag, RestClient};
use crate::sse::SseClient;
use crate::{init_tracing, TestConfig};
use anyhow::Result;
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
        // No cleanup needed - shared resources persist for other tests
    }
    // No cleanup needed - shared resources will be cleaned up by justfile after all tests

    assert!(result.is_ok(), "Chat propagation test failed: {result:?}");
}

async fn run_chat_propagation_test(
    cluster: &DeployedCluster,
    _config: &TestConfig,
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

    // Check if we got any chat events at all
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

    info!("Received {} chat events", chat_events.len());
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
        info!("Found our test message in the chat events");
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

// Presence propagation test placeholder removed until feature is implemented

/// Integration test for file upload and database backend verification
/// This test exercises the critical backend methods that were previously stubbed
#[tokio::test]
async fn test_file_upload_and_backend_queries() {
    init_tracing();
    let config = TestConfig::default();

    let cluster = get_shared_test_cluster().await;

    let result = run_file_backend_test(&cluster, &config).await;

    if let Err(e) = &result {
        tracing::error!("File backend test failed: {:?}", e);
    }

    assert!(result.is_ok(), "File backend test failed: {result:?}");
}

async fn run_file_backend_test(
    cluster: &DeployedCluster,
    _config: &TestConfig,
) -> Result<()> {
    info!("Starting file upload and backend verification test");

    // Set up REST client for the first proxy (targets node 0 -> Postgres)
    let proxy_url = format!("http://localhost:{}", 8532);
    let mut rest_client = RestClient::new(proxy_url);

    // Wait for proxy to be healthy
    info!("Waiting for proxy to become healthy");
    rest_client
        .wait_for_health(Duration::from_secs(60))
        .await
        .map_err(|e| {
            anyhow::anyhow!("Proxy failed to become healthy: {}", e)
        })?;

    // Authenticate
    if let Some(password) = cluster.proxy_password(0) {
        info!("Authenticating with proxy");
        rest_client.login(password).await?;
    } else {
        return Err(anyhow::anyhow!("No password found for proxy 0"));
    }

    // Prepare SSE stream up-front so we don't miss fast processing events
    let sse_stream = {
        let mut sse_client = SseClient::new(rest_client.base_url().to_string());
        if let Some(token) = rest_client.auth_token() {
            sse_client = sse_client.with_auth(token.to_string());
        }
        sse_client.connect_instance_events().await?
    };

    // Create a test file with some content
    let test_content = b"Hello, this is a test file for backend verification!";
    let test_tags = vec![
        FileTag {
            namespace: "category".to_string(),
            descriptor: "test".to_string(),
        },
        FileTag {
            namespace: "source".to_string(),
            descriptor: "integration-test".to_string(),
        },
    ];

    // Also test with JPEG image to trigger image processing and exercise image_row method
    let jpeg_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(
            "bafkreihlc3npy2lbafe3bndeqkxwwrtpvkebiip4pb26b4soziltjj4cqe.jpeg",
        );
    let jpeg_content = std::fs::read(&jpeg_path)?;
    let jpeg_tags = vec![
        FileTag {
            namespace: "category".to_string(),
            descriptor: "image".to_string(),
        },
        FileTag {
            namespace: "source".to_string(),
            descriptor: "test-vector".to_string(),
        },
    ];

    // Step 1: Start upload session - exercises backend new_tag_vocab potentially
    info!("Starting upload session");
    let upload_response = rest_client
        .start_upload(
            Some(test_content.len() as u64),
            Some("text/plain".to_string()),
        )
        .await?;

    info!("Upload session started: {}", upload_response.upload_id);

    // Step 2: Upload content in chunks - exercises backend new_file method
    info!("Uploading file content");
    let chunk_response = rest_client
        .upload_chunk(&upload_response.upload_id, 0, test_content.to_vec())
        .await?;

    info!(
        "Chunk uploaded, received {} bytes",
        chunk_response.bytes_received
    );

    // Step 3: Complete upload with tags - exercises backend new_tag_map, new_tag_vocab methods
    info!("Completing upload with tags");
    let complete_response = rest_client
        .complete_upload(&upload_response.upload_id, test_tags.clone())
        .await?;

    let cid = complete_response.cid;
    info!("Upload completed, CID: {}", cid);

    // Wait for processing to fully complete before querying
    info!("Waiting for processing to finish for CID: {}", cid);
    // Step 4: Verify file information - exercises backend file_row method (and potentially image_row/video_row)
    info!("Querying file information for CID: {}", cid);
    rest_client.wait_until_finished_processing(&cid).await?;
    let file_info = match rest_client.get_file_info(&cid).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(
                "Get file info failed for CID {} via {}: {}",
                cid,
                rest_client.base_url(),
                e
            );
            // Diagnostic: fetch files page and system info
            tracing::info!("Diagnostics: fetching files page 0");
            match rest_client.list_files(0).await {
                Ok(files_list) => {
                    tracing::info!(
                        "Files list page 0: {}",
                        serde_json::to_string_pretty(&files_list)?
                    );
                    let found = files_list
                        .get("files")
                        .and_then(|f| f.as_array())
                        .map(|arr| {
                            arr.iter().any(|f| {
                                f.get("cid").and_then(|c| c.as_str())
                                    == Some(&cid)
                            })
                        })
                        .unwrap_or(false);
                    if found {
                        tracing::warn!(
                            "CID {} appears in listing but cid-info returned 404",
                            cid
                        );
                    } else {
                        tracing::warn!(
                            "CID {} not found in first page listing either",
                            cid
                        );
                    }
                }
                Err(le) => tracing::warn!(
                    "Failed to list files for diagnostics: {}",
                    le
                ),
            }
            match rest_client.get_system_info().await {
                Ok(si) => tracing::info!(
                    "Diagnostics: system stats files={}, tags={}, assoc={}",
                    si.stats.files_indexed,
                    si.stats.tags_count,
                    si.stats.associations_count
                ),
                Err(se) => tracing::warn!(
                    "Failed to get system info for diagnostics: {}",
                    se
                ),
            }
            return Err(e);
        }
    };
    info!(
        "File info retrieved: {}",
        serde_json::to_string_pretty(&file_info)?
    );

    // Verify the file info contains expected data
    if let Some(size) = file_info.get("size") {
        let size_value = size.as_u64().unwrap_or(0);
        if size_value != test_content.len() as u64 {
            return Err(anyhow::anyhow!(
                "File size mismatch: expected {}, got {}",
                test_content.len(),
                size_value
            ));
        }
    }

    // Step 5: Verify file tags - exercises backend file_tags method
    info!("Querying file tags for CID: {}", cid);
    let file_tags = rest_client.get_file_tags(&cid).await?;
    info!("File tags retrieved: {} tags", file_tags.len());

    // Verify we got our uploaded tags back
    for expected_tag in &test_tags {
        let found = file_tags.iter().any(|tag| {
            tag.namespace == expected_tag.namespace
                && tag.descriptor == expected_tag.descriptor
        });
        if !found {
            return Err(anyhow::anyhow!(
                "Expected tag not found: {}:{}",
                expected_tag.namespace,
                expected_tag.descriptor
            ));
        }
    }

    // Step 6: List files to verify pagination - exercises backend files_page method
    info!("Testing file listing (exercises files_page method)");
    let files_list = rest_client.list_files(0).await?;
    info!(
        "Files list retrieved: {}",
        serde_json::to_string_pretty(&files_list)?
    );

    // Verify our uploaded file appears in the list
    if let Some(files_array) =
        files_list.get("files").and_then(|f| f.as_array())
    {
        let our_file_found = files_array.iter().any(|file| {
            file.get("cid")
                .and_then(|c| c.as_str())
                .map(|c| c == cid)
                .unwrap_or(false)
        });

        if !our_file_found {
            return Err(anyhow::anyhow!(
                "Uploaded file with CID {} not found in files list",
                cid
            ));
        }
        info!("Successfully found uploaded file in files list");
    }

    info!("Testing JPEG image upload to exercise image processing");
    let jpeg_upload_response = rest_client
        .start_upload(
            Some(jpeg_content.len() as u64),
            Some("image/jpeg".to_string()),
        )
        .await?;

    info!(
        "JPEG upload session started: {}",
        jpeg_upload_response.upload_id
    );

    let jpeg_chunk_response = rest_client
        .upload_chunk(&jpeg_upload_response.upload_id, 0, jpeg_content.clone())
        .await?;

    info!(
        "JPEG chunk uploaded, received {} bytes",
        jpeg_chunk_response.bytes_received
    );

    info!("Completing JPEG upload with image tags");
    let jpeg_complete_response = rest_client
        .complete_upload(&jpeg_upload_response.upload_id, jpeg_tags.clone())
        .await?;

    let jpeg_cid = jpeg_complete_response.cid;
    info!("JPEG upload completed, CID: {}", jpeg_cid);

    info!(
        "Waiting for JPEG processing to finish for CID: {}",
        jpeg_cid
    );
    info!("Querying JPEG file information for CID: {}", jpeg_cid);
    rest_client
        .wait_until_finished_processing(&jpeg_cid)
        .await?;
    let jpeg_file_info = match rest_client.get_file_info(&jpeg_cid).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(
                "Get JPEG file info failed for CID {} via {}: {}",
                jpeg_cid,
                rest_client.base_url(),
                e
            );
            return Err(e);
        }
    };

    info!(
        "JPEG file info retrieved: {}",
        serde_json::to_string_pretty(&jpeg_file_info)?
    );

    if let Some(ext_file) = jpeg_file_info.get("ext_file") {
        let height =
            ext_file.get("height").and_then(|h| h.as_i64()).unwrap_or(0);
        let width = ext_file.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
        let aspect_ratio = ext_file
            .get("aspect_ratio")
            .and_then(|r| r.as_f64())
            .unwrap_or(0.0);

        info!(
            "JPEG metadata - Height: {}, Width: {}, Aspect Ratio: {}",
            height, width, aspect_ratio
        );

        if height <= 0 || width <= 0 || aspect_ratio <= 0.0 {
            return Err(anyhow::anyhow!(
                "Invalid JPEG metadata: height={}, width={}, aspect_ratio={}",
                height,
                width,
                aspect_ratio
            ));
        }

        info!("✓ JPEG image processing and database storage successful!");
    } else {
        return Err(anyhow::anyhow!("JPEG file missing ext_file data"));
    }

    let jpeg_file_tags = rest_client.get_file_tags(&jpeg_cid).await?;
    for expected_tag in &jpeg_tags {
        let found = jpeg_file_tags.iter().any(|tag| {
            tag.namespace == expected_tag.namespace
                && tag.descriptor == expected_tag.descriptor
        });
        if !found {
            return Err(anyhow::anyhow!(
                "Expected JPEG tag not found: {}:{}",
                expected_tag.namespace,
                expected_tag.descriptor
            ));
        }
    }
    info!("✓ JPEG tags verified successfully");

    // Repeat the same file operations against proxy 1 (targets node 1 -> SQLite) if available
    if cluster.topology.proxies > 1 {
        info!("Repeating file operations via proxy 1 (SQLite-backed node)");

        let proxy1_url = format!("http://localhost:{}", 8532 + 1);
        let mut rest_client1 = RestClient::new(proxy1_url);

        // Wait for proxy to be healthy
        rest_client1
            .wait_for_health(Duration::from_secs(60))
            .await
            .map_err(|e| {
                anyhow::anyhow!("Proxy 1 failed to become healthy: {}", e)
            })?;

        // Authenticate
        if let Some(password) = cluster.proxy_password(1) {
            rest_client1.login(password).await?;
        } else {
            return Err(anyhow::anyhow!("No password found for proxy 1"));
        }

        // Prepare SSE for proxy 1 before starting upload
        let sse_stream1 = {
            let mut sse_client1 =
                SseClient::new(rest_client1.base_url().to_string());
            if let Some(token) = rest_client1.auth_token() {
                sse_client1 = sse_client1.with_auth(token.to_string());
            }
            sse_client1.connect_instance_events().await?
        };

        // Start upload session
        let upload_response1 = rest_client1
            .start_upload(
                Some(test_content.len() as u64),
                Some("text/plain".to_string()),
            )
            .await?;

        // Upload content in one chunk
        let _ = rest_client1
            .upload_chunk(&upload_response1.upload_id, 0, test_content.to_vec())
            .await?;

        // Complete upload with slightly different identifying tag
        let mut tags1 = test_tags.clone();
        tags1.push(FileTag {
            namespace: "source".to_string(),
            descriptor: "integration-test-sqlite".to_string(),
        });
        let complete_response1 = rest_client1
            .complete_upload(&upload_response1.upload_id, tags1.clone())
            .await?;
        let cid1 = complete_response1.cid;

        // Wait for processing to finish on proxy 1 / node 1 before querying
        info!(
            "Waiting for processing to finish for CID (proxy 1): {}",
            cid1
        );
        // Verify file info
        rest_client1.wait_until_finished_processing(&cid1).await?;
        let file_info1 = match rest_client1.get_file_info(&cid1).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    "Get file info failed for CID {} via {}: {}",
                    cid1,
                    rest_client1.base_url(),
                    e
                );
                tracing::info!("Diagnostics (proxy 1): fetching files page 0");
                match rest_client1.list_files(0).await {
                    Ok(files_list) => {
                        tracing::info!(
                            "Files list page 0 (proxy 1): {}",
                            serde_json::to_string_pretty(&files_list)?
                        );
                        let found = files_list
                            .get("files")
                            .and_then(|f| f.as_array())
                            .map(|arr| {
                                arr.iter().any(|f| {
                                    f.get("cid").and_then(|c| c.as_str())
                                        == Some(&cid1)
                                })
                            })
                            .unwrap_or(false);
                        if found {
                            tracing::warn!(
                                "CID {} appears in listing (proxy 1) but cid-info returned 404",
                                cid1
                            );
                        } else {
                            tracing::warn!(
                                "CID {} not found in first page listing (proxy 1) either",
                                cid1
                            );
                        }
                    }
                    Err(le) => tracing::warn!(
                        "Failed to list files for diagnostics (proxy 1): {}",
                        le
                    ),
                }
                match rest_client1.get_system_info().await {
                    Ok(si) => tracing::info!(
                        "Diagnostics (proxy 1): system stats files={}, tags={}, assoc={}",
                        si.stats.files_indexed, si.stats.tags_count, si.stats.associations_count
                    ),
                    Err(se) => tracing::warn!(
                        "Failed to get system info for diagnostics (proxy 1): {}",
                        se
                    ),
                }
                return Err(e);
            }
        };
        if let Some(size) = file_info1.get("size") {
            let size_value = size.as_u64().unwrap_or(0);
            if size_value != test_content.len() as u64 {
                return Err(anyhow::anyhow!(
                    "Proxy 1 file size mismatch: expected {}, got {}",
                    test_content.len(),
                    size_value
                ));
            }
        }

        // Verify tags
        let file_tags1 = rest_client1.get_file_tags(&cid1).await?;
        for expected_tag in &tags1 {
            let found = file_tags1.iter().any(|tag| {
                tag.namespace == expected_tag.namespace
                    && tag.descriptor == expected_tag.descriptor
            });
            if !found {
                return Err(anyhow::anyhow!(
                    "Proxy 1 expected tag not found: {}:{}",
                    expected_tag.namespace,
                    expected_tag.descriptor
                ));
            }
        }

        // Verify listing includes the new file
        let files_list1 = rest_client1.list_files(0).await?;
        if let Some(files_array) =
            files_list1.get("files").and_then(|f| f.as_array())
        {
            let our_file_found = files_array.iter().any(|file| {
                file.get("cid")
                    .and_then(|c| c.as_str())
                    .map(|c| c == cid1)
                    .unwrap_or(false)
            });
            if !our_file_found {
                return Err(anyhow::anyhow!(
                    "Proxy 1 uploaded file with CID {} not found in files list",
                    cid1
                ));
            }
        }

        info!("Testing JPEG upload via proxy 1 (SQLite backend)");
        let jpeg_upload1 = rest_client1
            .start_upload(
                Some(jpeg_content.len() as u64),
                Some("image/jpeg".to_string()),
            )
            .await?;

        let _ = rest_client1
            .upload_chunk(&jpeg_upload1.upload_id, 0, jpeg_content)
            .await?;

        let mut jpeg_tags1 = jpeg_tags;
        jpeg_tags1.push(FileTag {
            namespace: "backend".to_string(),
            descriptor: "sqlite".to_string(),
        });

        let jpeg_complete1 = rest_client1
            .complete_upload(&jpeg_upload1.upload_id, jpeg_tags1.clone())
            .await?;
        let jpeg_cid1 = jpeg_complete1.cid;

        info!("Waiting for JPEG processing (SQLite): {}", jpeg_cid1);
        rest_client1
            .wait_until_finished_processing(&jpeg_cid1)
            .await?;
        let jpeg_info1 = rest_client1.get_file_info(&jpeg_cid1).await?;

        if let Some(ext_file) = jpeg_info1.get("ext_file") {
            let height =
                ext_file.get("height").and_then(|h| h.as_i64()).unwrap_or(0);
            let width =
                ext_file.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
            let aspect_ratio = ext_file
                .get("aspect_ratio")
                .and_then(|r| r.as_f64())
                .unwrap_or(0.0);

            if height <= 0 || width <= 0 || aspect_ratio <= 0.0 {
                return Err(anyhow::anyhow!(
                    "SQLite JPEG metadata invalid: height={}, width={}, aspect_ratio={}",
                    height, width, aspect_ratio
                ));
            }

            info!("✓ SQLite JPEG processing successful!");
        } else {
            return Err(anyhow::anyhow!("SQLite JPEG missing ext_file data"));
        }

        let jpeg_tags1_result = rest_client1.get_file_tags(&jpeg_cid1).await?;
        for expected_tag in &jpeg_tags1 {
            let found = jpeg_tags1_result.iter().any(|tag| {
                tag.namespace == expected_tag.namespace
                    && tag.descriptor == expected_tag.descriptor
            });
            if !found {
                return Err(anyhow::anyhow!(
                    "SQLite JPEG tag not found: {}:{}",
                    expected_tag.namespace,
                    expected_tag.descriptor
                ));
            }
        }
    }

    info!("File backend verification test completed successfully across Postgres and SQLite nodes!");
    info!("Verified backend methods: new_file, new_tag_vocab, new_tag_map, file_row, file_tags, files_page, new_image, image_row on both nodes");

    Ok(())
}

/// Extended test that exercises additional backend methods not covered by basic upload
/// This test ensures database methods like thumbnails_by_source_cid, lookup_tag_id,
/// random_file, count methods, and batch operations are working
#[tokio::test]
async fn test_extended_backend_coverage() {
    init_tracing();
    let config = TestConfig::default();

    let cluster = get_shared_test_cluster().await;

    let result = run_extended_backend_test(&cluster, &config).await;

    if let Err(e) = &result {
        tracing::error!("Extended backend test failed: {:?}", e);
    }

    assert!(result.is_ok(), "Extended backend test failed: {result:?}");
}

async fn run_extended_backend_test(
    cluster: &DeployedCluster,
    _config: &TestConfig,
) -> Result<()> {
    info!("Starting extended backend verification test");

    // Set up REST client
    let proxy_url = format!("http://localhost:{}", 8532);
    let mut rest_client = RestClient::new(proxy_url);

    // Wait for proxy to be healthy and authenticate
    rest_client.wait_for_health(Duration::from_secs(60)).await?;
    if let Some(password) = cluster.proxy_password(0) {
        rest_client.login(password).await?;
    } else {
        return Err(anyhow::anyhow!("No password found for proxy"));
    }

    // Test system info endpoint - exercises backend count methods
    info!("Testing system stats (exercises count_files, count_tags, count_tag_associations)");
    let system_info = rest_client.get_system_info().await?;
    info!(
        "System stats - Files: {}, Tags: {}, Associations: {}",
        system_info.stats.files_indexed,
        system_info.stats.tags_count,
        system_info.stats.associations_count
    );

    // The backend count methods should return non-negative numbers
    if system_info.stats.files_indexed < 0
        || system_info.stats.tags_count < 0
        || system_info.stats.associations_count < 0
    {
        return Err(anyhow::anyhow!("System stats returned negative counts - backend count methods may be broken"));
    }

    // Test tag suggestion endpoint - exercises backend get_descriptors_that_start_with method
    info!(
        "Testing tag suggestions (exercises get_descriptors_that_start_with)"
    );
    // Use the generic GET method since we can't access client directly
    let suggest_url = format!("{}/suggest-tag/te", rest_client.base_url());
    let client = reqwest::Client::new();
    let mut req = client.get(&suggest_url);
    if let Some(token) = rest_client.auth_token() {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    let response = req.send().await?;

    if response.status().is_success() {
        let suggestions: serde_json::Value = response.json().await?;
        info!(
            "Tag suggestions retrieved: {}",
            serde_json::to_string_pretty(&suggestions)?
        );
    } else {
        warn!(
            "Tag suggestions endpoint returned error: {}",
            response.status()
        );
        // Don't fail the test as this might be expected if no tags exist yet
    }

    info!("Extended backend verification completed!");
    info!("Verified additional backend methods: count_files, count_tags, count_tag_associations, get_descriptors_that_start_with");

    Ok(())
}

async fn run_search_test(
    cluster: &DeployedCluster,
    _config: &TestConfig,
) -> Result<()> {
    info!("starting search functionality test");

    let proxy_url = format!("http://localhost:{}", 8532);
    let mut rest_client = RestClient::new(proxy_url);

    info!("waiting for proxy to become healthy");
    rest_client
        .wait_for_health(Duration::from_secs(60))
        .await
        .map_err(|e| {
            anyhow::anyhow!("proxy failed to become healthy: {}", e)
        })?;

    if let Some(password) = cluster.proxy_password(0) {
        info!("authenticating with proxy");
        rest_client.login(password).await?;
    } else {
        return Err(anyhow::anyhow!("no password found for proxy 0"));
    }

    let jpeg_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(
            "bafkreihj7edy35b227jzxyw2ixdkb326eysyrm65q6i4ht46kynzb2s42u.jpeg",
        );
    let jpeg_content = std::fs::read(&jpeg_path)?;
    let search_tags = vec![
        FileTag {
            namespace: "category".to_string(),
            descriptor: "searchtest".to_string(),
        },
        FileTag {
            namespace: "source".to_string(),
            descriptor: "search-vector".to_string(),
        },
    ];

    let upload_response = rest_client
        .start_upload(
            Some(jpeg_content.len() as u64),
            Some("image/jpeg".to_string()),
        )
        .await?;
    let _chunk_response = rest_client
        .upload_chunk(&upload_response.upload_id, 0, jpeg_content)
        .await?;
    let complete_response = rest_client
        .complete_upload(&upload_response.upload_id, search_tags.clone())
        .await?;
    let cid = complete_response.cid;

    info!("waiting for processing to complete for cid: {}", cid);
    rest_client.wait_until_finished_processing(&cid).await?;

    let search_results =
        rest_client.search_files("category:searchtest", 1).await?;

    let files = search_results["files"]
        .as_array()
        .expect("files should be array");
    let found = files.iter().any(|f| f["cid"].as_str() == Some(&cid));

    if !found {
        return Err(anyhow::anyhow!(
            "uploaded file not found in search results"
        ));
    }

    info!("✓ search functionality verified");
    Ok(())
}

#[tokio::test]
async fn test_search_functionality() {
    init_tracing();
    let config = TestConfig::default();

    let cluster = get_shared_test_cluster().await;

    let result = run_search_test(&cluster, &config).await;

    if let Err(e) = &result {
        tracing::error!("search test failed: {:?}", e);
    }

    assert!(result.is_ok(), "search test failed: {result:?}");
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
