use clap::{command, value_parser, Arg};
use dotenv::dotenv;
use hooya::proto::{
    control_server::{Control, ControlServer},
    ForgetFileReply, ForgetFileRequest, IndexFileReply, IndexFileRequest,
    StreamToFilestoreReply, StreamToFilestoreRequest, VersionReply,
    VersionRequest,
};
use std::{
    fs::create_dir_all,
    path::{Path, PathBuf},
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

    async fn stream_to_filestore(
        &self,
        _: Request<tonic::Streaming<StreamToFilestoreRequest>>,
    ) -> Result<Response<StreamToFilestoreReply>, Status> {
        let reply = StreamToFilestoreReply { cid: vec![] };

        Ok(Response::new(reply))
    }

    async fn index_file(
        &self,
        _: Request<IndexFileRequest>,
    ) -> Result<Response<IndexFileReply>, Status> {
        let reply = IndexFileReply {};

        Ok(Response::new(reply))
    }

    async fn forget_file(
        &self,
        _: Request<ForgetFileRequest>,
    ) -> Result<Response<ForgetFileReply>, Status> {
        let reply = ForgetFileReply {};

        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let xdg_pictures_dir = Path::new(env!("XDG_PICTURES_DIR"));
    let mut default_filestore_path = Path::new(".hooya").to_path_buf();
    if xdg_pictures_dir.is_dir() {
        default_filestore_path = xdg_pictures_dir.join("hooya");
    }

    let matches = command!()
        .arg(
            Arg::new("endpoint")
                .long("endpoint")
                .env("HOOYAD_ENDPOINT")
                .default_value(config::DEFAULT_HOOYAD_ENDPOINT),
        )
        .arg(
            Arg::new("filestore")
                .long("filestore")
                .env("HOOYAD_FILESTORE")
                .value_parser(value_parser!(PathBuf)),
        )
        .get_matches();

    let filestore_path = matches
        .get_one::<PathBuf>("filestore")
        .unwrap_or(&default_filestore_path);

    // Create filestore structure
    create_dir_all(filestore_path.join("store"))?;
    create_dir_all(filestore_path.join("forgotten"))?;
    create_dir_all(filestore_path.join("thumbs"))?;

    Server::builder()
        .add_service(ControlServer::new(IControl::default()))
        .serve(matches.get_one::<String>("endpoint").unwrap().parse()?)
        .await?;
    Ok(())
}
