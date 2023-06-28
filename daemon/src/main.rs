use hooya::proto::{
    control_server::{Control, ControlServer},
    VersionReply, VersionRequest,
};
use tonic::{transport::Server, Request, Response, Status};

#[derive(Debug, Default)]
pub struct IControl {}

#[tonic::async_trait]
impl Control for IControl {
    async fn version(
        &self,
        _: Request<VersionRequest>,
    ) -> Result<Response<VersionReply>, Status> {
        let reply = VersionReply {
            major_version: env!("CARGO_PKG_VERSION_MAJOR").to_string(),
            minor_version: env!("CARGO_PKG_VERSION_MINOR").to_string(),
            patch_version: env!("CARGO_PKG_VERSION_PATCH").to_string(),
        };

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;

    Server::builder()
        .add_service(ControlServer::new(IControl::default()))
        .serve(addr)
        .await?;
    Ok(())
}
