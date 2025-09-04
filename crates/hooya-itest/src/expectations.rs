use crate::sse::{ChatEvent, InstanceEvent, ProcessingEvent};
use anyhow::Result;
use std::time::Duration;
use tracing::{debug, info};

/// A collection of assertions for testing chat message propagation
pub struct ChatExpectations;

impl ChatExpectations {
    /// Assert that events contain a chat message from a specific node
    pub fn contains_message_from_node(
        events: &[InstanceEvent],
        node_id: &str,
        channel: &str,
        content: &str,
    ) -> Result<()> {
        let found = events.iter().any(|event| match event {
            InstanceEvent::Chat(chat) => {
                chat.node_id == node_id
                    && chat.channel == channel
                    && chat.content == content
            }
            _ => false,
        });

        if found {
            info!(
                "Found expected chat message from {} in {}: {}",
                node_id, channel, content
            );
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Expected chat message from {} in {} with content '{}' not found in {} events",
                node_id,
                channel,
                content,
                events.len()
            ))
        }
    }

    /// Assert that events contain any message in a channel
    pub fn contains_message_in_channel(
        events: &[InstanceEvent],
        channel: &str,
        content: &str,
    ) -> Result<()> {
        let found = events.iter().any(|event| match event {
            InstanceEvent::Chat(chat) => {
                chat.channel == channel && chat.content == content
            }
            _ => false,
        });

        if found {
            info!("Found expected message in {}: {}", channel, content);
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Expected message '{}' in channel '{}' not found in {} events",
                content,
                channel,
                events.len()
            ))
        }
    }

    /// Assert that events contain messages from multiple different nodes
    pub fn contains_messages_from_multiple_nodes(
        events: &[InstanceEvent],
        min_nodes: usize,
    ) -> Result<()> {
        let unique_nodes: std::collections::HashSet<_> = events
            .iter()
            .filter_map(|event| match event {
                InstanceEvent::Chat(chat) => Some(chat.node_id.as_str()),
                _ => None,
            })
            .collect();

        if unique_nodes.len() >= min_nodes {
            info!(
                "Found messages from {} different nodes (expected at least {})",
                unique_nodes.len(),
                min_nodes
            );
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Expected messages from at least {} nodes, found {} unique nodes: {:?}",
                min_nodes,
                unique_nodes.len(),
                unique_nodes
            ))
        }
    }

    /// Count chat events by channel
    pub fn count_events_by_channel(
        events: &[InstanceEvent],
    ) -> std::collections::HashMap<String, usize> {
        let mut counts = std::collections::HashMap::new();

        for event in events {
            if let InstanceEvent::Chat(chat) = event {
                *counts.entry(chat.channel.clone()).or_insert(0) += 1;
            }
        }

        debug!("Event counts by channel: {:?}", counts);
        counts
    }

    /// Extract all chat events from mixed events
    pub fn extract_chat_events(events: &[InstanceEvent]) -> Vec<&ChatEvent> {
        events
            .iter()
            .filter_map(|event| match event {
                InstanceEvent::Chat(chat) => Some(chat),
                _ => None,
            })
            .collect()
    }
}

/// A collection of assertions for testing processing events
pub struct ProcessingExpectations;

impl ProcessingExpectations {
    /// Assert that events contain a processing started event for a CID
    pub fn contains_processing_started(
        events: &[InstanceEvent],
        cid: &str,
    ) -> Result<()> {
        let found = events.iter().any(|event| match event {
            InstanceEvent::ProcessingStarted(pe) => pe.cid == cid,
            _ => false,
        });

        if found {
            info!("Found processing started event for CID: {}", cid);
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Processing started event for CID '{}' not found in {} events",
                cid,
                events.len()
            ))
        }
    }

    /// Assert that events contain a processing finished event for a CID
    pub fn contains_processing_finished(
        events: &[InstanceEvent],
        cid: &str,
    ) -> Result<()> {
        let found = events.iter().any(|event| match event {
            InstanceEvent::ProcessingFinished(pe) => pe.cid == cid,
            _ => false,
        });

        if found {
            info!("Found processing finished event for CID: {}", cid);
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Processing finished event for CID '{}' not found in {} events",
                cid,
                events.len()
            ))
        }
    }

    /// Extract all processing events from mixed events
    pub fn extract_processing_events(
        events: &[InstanceEvent],
    ) -> Vec<&ProcessingEvent> {
        events
            .iter()
            .filter_map(|event| match event {
                InstanceEvent::ProcessingStarted(pe)
                | InstanceEvent::ProcessingFinished(pe)
                | InstanceEvent::ProcessingFailed(pe)
                | InstanceEvent::ThumbnailGenerated(pe)
                | InstanceEvent::VideoPreviewGenerated(pe) => Some(pe),
                _ => None,
            })
            .collect()
    }
}

/// Helper for building expectation chains
pub struct ExpectationBuilder {
    errors: Vec<anyhow::Error>,
}

impl ExpectationBuilder {
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Add a chat expectation
    pub fn expect_chat_from_node(
        mut self,
        events: &[InstanceEvent],
        node_id: &str,
        channel: &str,
        content: &str,
    ) -> Self {
        if let Err(e) = ChatExpectations::contains_message_from_node(
            events, node_id, channel, content,
        ) {
            self.errors.push(e);
        }
        self
    }

    /// Add a processing expectation
    pub fn expect_processing_started(
        mut self,
        events: &[InstanceEvent],
        cid: &str,
    ) -> Self {
        if let Err(e) =
            ProcessingExpectations::contains_processing_started(events, cid)
        {
            self.errors.push(e);
        }
        self
    }

    /// Add a minimum node count expectation
    pub fn expect_messages_from_nodes(
        mut self,
        events: &[InstanceEvent],
        min_nodes: usize,
    ) -> Self {
        if let Err(e) = ChatExpectations::contains_messages_from_multiple_nodes(
            events, min_nodes,
        ) {
            self.errors.push(e);
        }
        self
    }

    /// Build and return result - succeeds only if all expectations passed
    pub fn build(self) -> Result<()> {
        if self.errors.is_empty() {
            info!("All expectations passed");
            Ok(())
        } else {
            let error_msgs: Vec<String> =
                self.errors.iter().map(|e| e.to_string()).collect();
            Err(anyhow::anyhow!(
                "Expectation failures:\n{}",
                error_msgs.join("\n")
            ))
        }
    }
}

/// Utility functions for common test patterns
pub struct TestUtils;

impl TestUtils {
    /// Wait for events and assert they meet expectations within timeout
    pub async fn wait_and_assert<F>(
        stream: &mut crate::sse::SseEventStream,
        timeout: Duration,
        assertion: F,
    ) -> Result<Vec<InstanceEvent>>
    where
        F: Fn(&[InstanceEvent]) -> Result<()>,
    {
        let events = stream.collect_events(timeout).await;
        assertion(&events)?;
        Ok(events)
    }

    /// Collect events from multiple streams concurrently
    pub async fn collect_from_multiple_streams(
        streams: &mut [crate::sse::SseEventStream],
        timeout: Duration,
    ) -> Vec<Vec<InstanceEvent>> {
        let mut results = Vec::new();

        // TODO: This could be made concurrent with join_all, but for now we'll do sequential
        for stream in streams {
            let events = stream.collect_events(timeout).await;
            results.push(events);
        }

        results
    }

    /// Log event summary for debugging
    pub fn log_event_summary(events: &[InstanceEvent]) {
        let chat_count = events
            .iter()
            .filter(|e| matches!(e, InstanceEvent::Chat(_)))
            .count();
        let processing_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    InstanceEvent::ProcessingStarted(_)
                        | InstanceEvent::ProcessingFinished(_)
                        | InstanceEvent::ProcessingFailed(_)
                        | InstanceEvent::ThumbnailGenerated(_)
                        | InstanceEvent::VideoPreviewGenerated(_)
                )
            })
            .count();
        let unknown_count = events
            .iter()
            .filter(|e| matches!(e, InstanceEvent::Unknown(_, _)))
            .count();

        info!(
            "Event summary: {} total ({} chat, {} processing, {} unknown)",
            events.len(),
            chat_count,
            processing_count,
            unknown_count
        );

        // Log unique node IDs in chat events
        let unique_chat_nodes: std::collections::HashSet<_> = events
            .iter()
            .filter_map(|e| match e {
                InstanceEvent::Chat(chat) => Some(&chat.node_id),
                _ => None,
            })
            .collect();

        if !unique_chat_nodes.is_empty() {
            debug!("Unique chat nodes: {:?}", unique_chat_nodes);
        }
    }
}

impl Default for ExpectationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_chat_event(
        node_id: &str,
        channel: &str,
        content: &str,
    ) -> InstanceEvent {
        InstanceEvent::Chat(ChatEvent {
            channel: channel.to_string(),
            content: content.to_string(),
            node_id: node_id.to_string(),
            signature: "test_sig".to_string(),
        })
    }

    #[test]
    fn test_contains_message_from_node() {
        let events = vec![
            create_test_chat_event("node1", "general", "hello"),
            create_test_chat_event("node2", "general", "world"),
        ];

        assert!(ChatExpectations::contains_message_from_node(
            &events, "node1", "general", "hello"
        )
        .is_ok());

        assert!(ChatExpectations::contains_message_from_node(
            &events, "node3", "general", "hello"
        )
        .is_err());
    }

    #[test]
    fn test_contains_messages_from_multiple_nodes() {
        let events = vec![
            create_test_chat_event("node1", "general", "hello"),
            create_test_chat_event("node2", "general", "world"),
            create_test_chat_event("node3", "general", "test"),
        ];

        assert!(ChatExpectations::contains_messages_from_multiple_nodes(
            &events, 2
        )
        .is_ok());
        assert!(ChatExpectations::contains_messages_from_multiple_nodes(
            &events, 3
        )
        .is_ok());
        assert!(ChatExpectations::contains_messages_from_multiple_nodes(
            &events, 4
        )
        .is_err());
    }

    #[test]
    fn test_expectation_builder() {
        let events = vec![
            create_test_chat_event("node1", "general", "hello"),
            create_test_chat_event("node2", "general", "world"),
        ];

        let result = ExpectationBuilder::new()
            .expect_chat_from_node(&events, "node1", "general", "hello")
            .expect_messages_from_nodes(&events, 2)
            .build();

        assert!(result.is_ok());

        let result = ExpectationBuilder::new()
            .expect_chat_from_node(&events, "node3", "general", "missing")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_count_events_by_channel() {
        let events = vec![
            create_test_chat_event("node1", "general", "hello"),
            create_test_chat_event("node2", "general", "world"),
            create_test_chat_event("node1", "random", "test"),
        ];

        let counts = ChatExpectations::count_events_by_channel(&events);
        assert_eq!(counts.get("general"), Some(&2));
        assert_eq!(counts.get("random"), Some(&1));
        assert_eq!(counts.get("nonexistent"), None);
    }
}
