use crate::chatroom::Chatroom;
use crate::mesh::{ChatMessage, MeshMessage};
use crate::mesh_network::MessageHandler;
use crate::runtime::InstanceEvent;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

pub struct ChatMessageHandler {
    async_tx: mpsc::Sender<(String, String, String, Vec<u8>)>, // (channel, sender, content, signature)
}

impl ChatMessageHandler {
    pub fn new(
        chatroom: Arc<Chatroom>,
        instance_events: broadcast::Sender<InstanceEvent>,
    ) -> Self {
        let (async_tx, mut async_rx) =
            mpsc::channel::<(String, String, String, Vec<u8>)>(1000);

        // spawn async task to handle chat logging and event broadcasting
        let chatroom_clone = chatroom.clone();
        tokio::spawn(async move {
            while let Some((channel, sender, content, signature)) =
                async_rx.recv().await
            {
                // log the message to file
                if let Err(e) = chatroom_clone
                    .log_message(&channel, &sender, &content)
                    .await
                {
                    eprintln!("Failed to log chat message: {e}");
                }

                // broadcast the chat event
                let formatted_content =
                    crate::format_chat_message(&sender, &content);
                let chat_event = crate::runtime::ChatEvent {
                    channel: channel.clone(),
                    content: formatted_content,
                    node_id: sender.clone(),
                    signature: signature.clone(),
                };

                let instance_event = InstanceEvent {
                    event_type: crate::runtime::InstanceEventType::Chat(
                        chat_event,
                    ),
                };

                if let Err(e) = instance_events.send(instance_event) {
                    eprintln!("error broadcasting instance event: {}", e);
                }
            }
        });

        Self { async_tx }
    }
}

impl MessageHandler for ChatMessageHandler {
    fn validate_message(&self, message: &MeshMessage) -> bool {
        // only handle chat messages
        if let Some(ref payload) = message.payload {
            match payload {
                crate::mesh::mesh_message::Payload::Chat(ref chat_msg) => {
                    return self.validate_chat_message(chat_msg);
                }
            }
        }
        false
    }

    fn handle_message(&self, message: MeshMessage) -> Result<()> {
        println!("ChatMessageHandler::handle_message called");
        if let Some(payload) = message.payload {
            match payload {
                crate::mesh::mesh_message::Payload::Chat(chat_msg) => {
                    // send to async channel for processing
                    if self
                        .async_tx
                        .try_send((
                            chat_msg.channel,
                            message.sender_node_id,
                            chat_msg.content,
                            message.signature,
                        ))
                        .is_err()
                    {
                        eprintln!(
                            "Chat handler async channel full, dropping message"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn get_subscribed_topics(&self) -> Vec<String> {
        vec![
            "/chat/room/general".to_string(),
            "/chat/room/dev".to_string(),
        ]
    }
}

impl ChatMessageHandler {
    /// Validate chat message content
    fn validate_chat_message(&self, chat_msg: &ChatMessage) -> bool {
        // validate channel name
        if chat_msg.channel.is_empty() || chat_msg.channel.len() > 64 {
            return false;
        }

        // check for invalid characters in channel name
        if !chat_msg
            .channel
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return false;
        }

        // validate content length
        if chat_msg.content.is_empty() || chat_msg.content.len() > 4096 {
            return false;
        }

        true
    }
}
