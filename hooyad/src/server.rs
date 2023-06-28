use hooya::proto::{
    control_server::{Control, ControlServer},
    VersionReply, VersionRequest,
};
use tonic::{transport::Server, Request, Response, Status};

mod config;

#[derive(Debug, Default)]
pub struct IControl {}

#[tonic::async_trait]
impl Control for IControl {
    async fn version(
        &self,
        _: Request<VersionRequest>,
    ) -> Result<Response<VersionReply>, Status> {
        let reply = VersionReply {
            major_version: env!("CARGO_PKG_VERSION_MAJOR")
                .parse::<u64>()
                .unwrap(),
            minor_version: env!("CARGO_PKG_VERSION_MINOR")
                .parse::<u64>()
                .unwrap(),
            patch_version: env!("CARGO_PKG_VERSION_PATCH")
                .parse::<u64>()
                .unwrap(),
            pre_version: env!("CARGO_PKG_VERSION_PRE").to_string(),
        };

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::Config::from_env_and_args();
    let endpoint = cfg.endpoint.parse()?;

    Server::builder()
        .add_service(ControlServer::new(IControl::default()))
        .serve(endpoint)
        .await?;
    Ok(())
}