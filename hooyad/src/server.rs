use clap::{command, value_parser, Arg};
use dotenv::dotenv;
use hooya::proto::{
    control_server::{Control, ControlServer},
    FileChunk, ForgetFileReply, ForgetFileRequest, IndexFileReply,
    IndexFileRequest, StreamToFilestoreReply, VersionReply, VersionRequest,
};
use rand::distributions::DistString;
use ring::digest::{Context, SHA256};
use std::{
    fs::{create_dir_all, File},
    io::Write,
    path::{Path, PathBuf},
};
use tokio_stream::StreamExt;
use tonic::{transport::Server, Request, Response, Status};

mod config;

#[derive(Debug, Default)]
pub struct IControl {
    filestore_path: PathBuf,
}

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
        r: Request<tonic::Streaming<FileChunk>>,
    ) -> Result<Response<StreamToFilestoreReply>, Status> {
        let mut chunk_stream = r.into_inner();
        let mut sha_context = Context::new(&SHA256);

        let tmp_name = rand::distributions::Alphanumeric
            .sample_string(&mut rand::thread_rng(), 16);
        let tmp_path = self.filestore_path.join("tmp").join(tmp_name);
        let mut fh = File::create(tmp_path.clone())?;

        while let Some(res) = chunk_stream.next().await {
            let data = &res?.data;
            // Feed chunk to SHA2-256 algorithm
            sha_context.update(data);
            // Append to on-disk file
            fh.write_all(data)?;
        }

        let cid = hooya::cid::wrap_digest(sha_context.finish())
            .map_err(|e| Status::internal(e.to_string()))?;
        let encoded_cid = hooya::cid::encode(&cid);

        let final_dir =
            self.filestore_path.join("store").join(&encoded_cid[..6]);
        let final_path = final_dir.join(encoded_cid);
        if !final_dir.is_dir() {
            std::fs::create_dir(final_dir)?;
        }
        std::fs::rename(tmp_path, final_path)?;
        let reply = StreamToFilestoreReply { cid };
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

    // Derive a path to use as filestore
    let mut default_filestore_path = Path::new(".hooya").to_path_buf();
    if let Ok(xdg_pictures_dir) = std::env::var("XDG_PICTURES_DIR") {
        let xdg_pictures_path = Path::new(&xdg_pictures_dir);
        if xdg_pictures_path.is_dir() {
            default_filestore_path = xdg_pictures_path.join("hooya");
        }
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
    create_dir_all(filestore_path.join("tmp"))?;

    Server::builder()
        .add_service(ControlServer::new(IControl {
            filestore_path: filestore_path.to_path_buf(),
        }))
        .serve(matches.get_one::<String>("endpoint").unwrap().parse()?)
        .await?;
    Ok(())
}
