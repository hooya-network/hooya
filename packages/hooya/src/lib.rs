pub mod proto {
    tonic::include_proto!("hooya");
}

mod chunked_reader;

pub use chunked_reader::*;

pub mod cid;
pub mod client;
pub mod image;
pub mod local;
pub mod runtime;

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

impl ToString for proto::Tag {
    fn to_string(&self) -> String {
        vec![self.namespace.clone(), self.descriptor.clone()].join(":")
    }
}

