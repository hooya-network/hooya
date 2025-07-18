pub mod proto {
    tonic::include_proto!("hooya");
}

pub mod mesh {
    tonic::include_proto!("hooya.mesh");
}

mod chunked_reader;

pub use chunked_reader::*;

pub mod addr_book;
pub mod chat_handler;
pub mod chatroom;
pub mod cid;
pub mod client;
pub mod image;
pub mod keys;
pub mod local;
pub mod mesh_network;
pub mod runtime;
pub mod video;
pub mod visibility;

impl From<&str> for proto::Tag {
    fn from(tag_str: &str) -> Self {
        let (namespace, descriptor) =
            tag_str.split_once(':').unwrap_or(("general", tag_str));
        proto::Tag {
            namespace: namespace.to_string(),
            descriptor: descriptor.to_string(),
        }
    }
}

impl std::fmt::Display for proto::Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.descriptor)
    }
}

/// Format a chat message for display and logging
/// Returns: "HH:MM  <sender> content"
pub fn format_chat_message(sender: &str, content: &str) -> String {
    let timestamp = chrono::Local::now().format("%H:%M");
    format!("{timestamp}  <{sender}> {content}")
}
