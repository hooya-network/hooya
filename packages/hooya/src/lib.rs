pub mod proto {
    tonic::include_proto!("hooya");
}

pub mod mesh {
    tonic::include_proto!("hooya.mesh");
}

mod chunked_reader;

pub use chunked_reader::*;

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
