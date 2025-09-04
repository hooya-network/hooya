use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    pub channel: String,
    pub content: String,
    pub node_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingEvent {
    pub cid: String,
    pub long_edge: Option<u32>,
    pub mimetype: Option<String>,
}

#[derive(Debug, Clone)]
pub enum InstanceEvent {
    Chat(ChatEvent),
    ProcessingStarted(ProcessingEvent),
    ProcessingFinished(ProcessingEvent),
    ProcessingFailed(ProcessingEvent),
    ThumbnailGenerated(ProcessingEvent),
    VideoPreviewGenerated(ProcessingEvent),
    Unknown(String, String), // event_type, data
}

pub struct SseClient {
    client: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl SseClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            auth_token: None,
        }
    }

    pub fn with_auth(mut self, token: String) -> Self {
        self.auth_token = Some(token);
        self
    }

    /// Connect to the instance events SSE endpoint
    pub async fn connect_instance_events(&self) -> Result<SseEventStream> {
        let url = format!("{}/api/events/instance", self.base_url);

        let mut request = self.client.get(&url);

        if let Some(token) = &self.auth_token {
            request =
                request.header("Authorization", format!("Bearer {token}"));
        }

        let response = request
            .send()
            .await
            .context("Failed to connect to SSE endpoint")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "SSE connection failed with status: {}",
                response.status()
            ));
        }

        info!("Connected to SSE endpoint: {}", url);

        let stream =
            response
                .bytes_stream()
                .eventsource()
                .map(|result| match result {
                    Ok(event) => {
                        debug!("Received SSE event: {:?}", event);
                        parse_sse_event(event)
                    }
                    Err(e) => {
                        error!("SSE stream error: {}", e);
                        Err(anyhow::anyhow!("SSE stream error: {}", e))
                    }
                });

        Ok(SseEventStream {
            stream: Box::pin(stream),
        })
    }

    /// Connect with auth query parameter (for unauthenticated access to processing events)
    pub async fn connect_instance_events_with_query_auth(
        &self,
        token: &str,
    ) -> Result<SseEventStream> {
        let url =
            format!("{}/api/events/instance?auth={}", self.base_url, token);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to SSE endpoint with query auth")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "SSE connection failed with status: {}",
                response.status()
            ));
        }

        info!("Connected to SSE endpoint with query auth: {}", url);

        let stream =
            response
                .bytes_stream()
                .eventsource()
                .map(|result| match result {
                    Ok(event) => parse_sse_event(event),
                    Err(e) => {
                        error!("SSE stream error: {}", e);
                        Err(anyhow::anyhow!("SSE stream error: {}", e))
                    }
                });

        Ok(SseEventStream {
            stream: Box::pin(stream),
        })
    }
}

pub struct SseEventStream {
    stream: Pin<Box<dyn Stream<Item = Result<InstanceEvent>> + Send>>,
}

impl SseEventStream {
    /// Collect events for a specified duration
    pub async fn collect_events(
        &mut self,
        duration: Duration,
    ) -> Vec<InstanceEvent> {
        let mut events = Vec::new();
        let timeout = tokio::time::sleep(duration);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                event_result = self.stream.next() => {
                    match event_result {
                        Some(Ok(event)) => {
                            debug!("Collected event: {:?}", event);
                            events.push(event);
                        }
                        Some(Err(e)) => {
                            warn!("Error collecting event: {}", e);
                            break;
                        }
                        None => {
                            debug!("SSE stream ended");
                            break;
                        }
                    }
                }
                _ = &mut timeout => {
                    debug!("Collection timeout reached, collected {} events", events.len());
                    break;
                }
            }
        }

        events
    }

    /// Wait for the first event matching a predicate
    pub async fn wait_for_event<F>(
        &mut self,
        predicate: F,
        timeout: Duration,
    ) -> Result<InstanceEvent>
    where
        F: Fn(&InstanceEvent) -> bool,
    {
        let timeout_future = tokio::time::sleep(timeout);
        tokio::pin!(timeout_future);

        loop {
            tokio::select! {
                event_result = self.stream.next() => {
                    match event_result {
                        Some(Ok(event)) => {
                            if predicate(&event) {
                                debug!("Found matching event: {:?}", event);
                                return Ok(event);
                            }
                        }
                        Some(Err(e)) => {
                            return Err(e);
                        }
                        None => {
                            return Err(anyhow::anyhow!("SSE stream ended"));
                        }
                    }
                }
                _ = &mut timeout_future => {
                    return Err(anyhow::anyhow!("Timeout waiting for event"));
                }
            }
        }
    }

    /// Wait for a chat message from a specific node
    pub async fn wait_for_chat_from_node(
        &mut self,
        node_id: &str,
        channel: &str,
        timeout: Duration,
    ) -> Result<ChatEvent> {
        let node_id = node_id.to_string();
        let channel = channel.to_string();

        let event = self
            .wait_for_event(
                |event| match event {
                    InstanceEvent::Chat(chat) => {
                        chat.node_id == node_id && chat.channel == channel
                    }
                    _ => false,
                },
                timeout,
            )
            .await?;

        match event {
            InstanceEvent::Chat(chat) => Ok(chat),
            _ => unreachable!(),
        }
    }

    /// Wait for any chat message in a channel
    pub async fn wait_for_chat_in_channel(
        &mut self,
        channel: &str,
        timeout: Duration,
    ) -> Result<ChatEvent> {
        let channel = channel.to_string();

        let event = self
            .wait_for_event(
                |event| match event {
                    InstanceEvent::Chat(chat) => chat.channel == channel,
                    _ => false,
                },
                timeout,
            )
            .await?;

        match event {
            InstanceEvent::Chat(chat) => Ok(chat),
            _ => unreachable!(),
        }
    }

    /// Filter events by type during collection
    pub async fn collect_chat_events(
        &mut self,
        duration: Duration,
    ) -> Vec<ChatEvent> {
        let events = self.collect_events(duration).await;
        events
            .into_iter()
            .filter_map(|event| match event {
                InstanceEvent::Chat(chat) => Some(chat),
                _ => None,
            })
            .collect()
    }

    pub async fn collect_processing_events(
        &mut self,
        duration: Duration,
    ) -> Vec<ProcessingEvent> {
        let events = self.collect_events(duration).await;
        events
            .into_iter()
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

    /// Wait until a specific CID finishes processing (or fails)
    /// Returns the ProcessingEvent on success, or an error if it fails or times out
    pub async fn wait_for_processing_finished(
        &mut self,
        target_cid: &str,
        timeout: Duration,
    ) -> Result<ProcessingEvent> {
        let cid = target_cid.to_string();
        let event = self
            .wait_for_event(
                |event| match event {
                    InstanceEvent::ProcessingFinished(pe) if pe.cid == cid => {
                        true
                    }
                    InstanceEvent::ProcessingFailed(pe) if pe.cid == cid => {
                        true
                    }
                    _ => false,
                },
                timeout,
            )
            .await?;

        match event {
            InstanceEvent::ProcessingFinished(pe) => Ok(pe),
            InstanceEvent::ProcessingFailed(pe) => Err(anyhow::anyhow!(
                "Processing failed for CID {} (mimetype={:?})",
                pe.cid,
                pe.mimetype
            )),
            _ => unreachable!(),
        }
    }
}

fn parse_sse_event(event: eventsource_stream::Event) -> Result<InstanceEvent> {
    let event_type = event.event;
    let data = event.data;

    debug!("Parsing SSE event: type={}, data={}", event_type, data);

    match event_type.as_str() {
        "chat_message" => {
            let chat_event: ChatEvent = serde_json::from_str(&data)
                .context("Failed to parse chat event")?;
            Ok(InstanceEvent::Chat(chat_event))
        }
        "processing_started" => {
            let processing_event: ProcessingEvent = serde_json::from_str(&data)
                .context("Failed to parse processing started event")?;
            Ok(InstanceEvent::ProcessingStarted(processing_event))
        }
        "processing_finished" => {
            let processing_event: ProcessingEvent = serde_json::from_str(&data)
                .context("Failed to parse processing finished event")?;
            Ok(InstanceEvent::ProcessingFinished(processing_event))
        }
        "processing_failed" => {
            let processing_event: ProcessingEvent = serde_json::from_str(&data)
                .context("Failed to parse processing failed event")?;
            Ok(InstanceEvent::ProcessingFailed(processing_event))
        }
        "thumbnail_generated" => {
            let processing_event: ProcessingEvent = serde_json::from_str(&data)
                .context("Failed to parse thumbnail generated event")?;
            Ok(InstanceEvent::ThumbnailGenerated(processing_event))
        }
        "video_preview_generated" => {
            let processing_event: ProcessingEvent = serde_json::from_str(&data)
                .context("Failed to parse video preview generated event")?;
            Ok(InstanceEvent::VideoPreviewGenerated(processing_event))
        }
        _ => {
            warn!("Unknown SSE event type: {}", event_type);
            Ok(InstanceEvent::Unknown(event_type, data))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_chat_event() {
        let event = eventsource_stream::Event {
            event: "chat_message".to_string(),
            data: r#"{"channel":"general","content":"hello world","node_id":"0x123","signature":"sig123"}"#.to_string(),
            id: "".to_string(),
            retry: None,
        };

        let parsed = parse_sse_event(event).unwrap();
        match parsed {
            InstanceEvent::Chat(chat) => {
                assert_eq!(chat.channel, "general");
                assert_eq!(chat.content, "hello world");
                assert_eq!(chat.node_id, "0x123");
                assert_eq!(chat.signature, "sig123");
            }
            _ => panic!("Expected chat event"),
        }
    }

    #[test]
    fn test_parse_processing_event() {
        let event = eventsource_stream::Event {
            event: "processing_started".to_string(),
            data: r#"{"cid":"QmTest123"}"#.to_string(),
            id: "".to_string(),
            retry: None,
        };

        let parsed = parse_sse_event(event).unwrap();
        match parsed {
            InstanceEvent::ProcessingStarted(pe) => {
                assert_eq!(pe.cid, "QmTest123");
            }
            _ => panic!("Expected processing started event"),
        }
    }
}
