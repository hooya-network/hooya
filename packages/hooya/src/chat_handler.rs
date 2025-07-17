use crate::chatroom::Chatroom;
use crate::mesh::{ChatMessage, MeshMessage};
use crate::mesh_network::MessageHandler;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct ChatMessageHandler {
    async_tx: mpsc::Sender<(String, String, String)>, // (channel, sender, content)
}

impl ChatMessageHandler {
    pub fn new(chatroom: Arc<Chatroom>) -> Self {
        let (async_tx, mut async_rx) =
            mpsc::channel::<(String, String, String)>(1000);

        // spawn async task to handle chat logging
        let chatroom_clone = chatroom.clone();
        tokio::spawn(async move {
            while let Some((channel, sender, content)) = async_rx.recv().await {
                if let Err(e) = chatroom_clone
                    .log_message(&channel, &sender, &content)
                    .await
                {
                    eprintln!("Failed to log chat message: {e}");
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
