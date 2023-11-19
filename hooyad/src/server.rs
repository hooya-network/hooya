use clap::{command, value_parser, Arg};
use dotenv::dotenv;
use futures_util::Stream;
use hooya::proto::{
    control_server::{Control, ControlServer},
    CidInfoReply, CidInfoRequest, CidThumbnailRequest, ContentAtCidRequest,
    FileChunk, ForgetFileReply, ForgetFileRequest, LocalFilePageReply,
    LocalFilePageRequest, RandomLocalFileReply, RandomLocalFileRequest,
    StreamToFilestoreReply, TagCidReply, TagCidRequest, TagsReply, TagsRequest,
    VersionReply, VersionRequest,
};
use hooya::runtime::Runtime;
use rand::distributions::DistString;
use sqlx::migrate::MigrateDatabase;
use sqlx::{Sqlite, SqlitePool};
use std::{
    fs::{create_dir_all, File},
    io::Write,
    path::{Path, PathBuf},
    pin::Pin,
};
use tokio_stream::StreamExt;
use tonic::{transport::Server, Request, Response, Status};

mod config;

struct IControl {
    pub runtime: Runtime,
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
        let runtime = &self.runtime;
        let mut chunk_stream = r.into_inner();
        let mut sha_context = hooya::cid::new_digest_context();

        let tmp_name = rand::distributions::Alphanumeric
            .sample_string(&mut rand::thread_rng(), 16);
        let tmp_path = runtime.filestore_path.join("tmp").join(tmp_name);
        let mut fh = File::create(tmp_path.clone())?;

        while let Some(res) = chunk_stream.next().await {
            let data = &res?.data;
            // Feed chunk to SHA2-256 algorithm
            sha_context.update(data);
            // Append to on-disk file
            fh.write_all(data)?;
        }

        let len = fh.metadata()?.len();

        if len == 0 {
            return Err(Status::invalid_argument("Empty file"));
        }

        let cid = hooya::cid::wrap_digest(sha_context.finish())
            .map_err(|e| Status::internal(e.to_string()))?;
        let cid_store_path = runtime.derive_store_path(&cid).unwrap();

        // I know this always has a parent so .unwrap() okie
        let parent = cid_store_path.parent().unwrap();

        if !parent.is_dir() {
            std::fs::create_dir(parent)?;
        }
        std::fs::rename(tmp_path, cid_store_path)?;

        self.runtime
            .new_from_filestore(cid.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let reply = StreamToFilestoreReply { cid };
        Ok(Response::new(reply))
    }

    async fn tag_cid(
        &self,
        r: Request<TagCidRequest>,
    ) -> Result<Response<TagCidReply>, Status> {
        let runtime = &self.runtime;
        let req = r.into_inner();

        let reply = TagCidReply {};

        // Check that the CID is actually indexed before tagging it
        runtime.indexed_file(req.cid.clone()).await.map_err(|_| {
            Status::internal("CID is not indexed so it cannot be tagged")
        })?;

        runtime
            .tag_cid(req.cid, req.tags)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(reply))
    }

    type ContentAtCidStream =
        Pin<Box<dyn Stream<Item = Result<FileChunk, Status>> + Send + 'static>>;
    async fn content_at_cid(
        &self,
        r: Request<ContentAtCidRequest>,
    ) -> Result<Response<Self::ContentAtCidStream>, Status> {
        let cid = r.into_inner().cid;

        // NOTE this is safe because we are in charge of encoding the binary
        // data and the set of characters in base32 cannot be used for
        // malicious dir traversal
        let local_file = self
            .runtime
            .derive_store_path(&cid)
            .map_err(|e| Status::internal(e.to_string()))?;
        let fh = File::open(local_file)?;

        let chunks = hooya::ChunkedReader::new(fh);
        let stream = tokio_stream::iter(chunks).map(move |c| {
            let data = c?;
            Ok(FileChunk { data })
        });

        Ok(Response::new(Box::pin(stream)))
    }

    type CidThumbnailStream =
        Pin<Box<dyn Stream<Item = Result<FileChunk, Status>> + Send + 'static>>;
    async fn cid_thumbnail(
        &self,
        r: Request<CidThumbnailRequest>,
    ) -> Result<Response<Self::CidThumbnailStream>, Status> {
        let req = r.into_inner();

        // NOTE this is safe because we are in charge of encoding the binary
        // data and the set of characters in base32 cannot be used for
        // malicious dir traversal
        let local_file = self
            .runtime
            .derive_thumb_path(&req.source_cid, req.long_edge)
            .map_err(|e| Status::internal(e.to_string()))?;
        let fh = File::open(local_file)?;

        let chunks = hooya::ChunkedReader::new(fh);
        let stream = tokio_stream::iter(chunks).map(move |c| {
            let data = c?;
            Ok(FileChunk { data })
        });

        Ok(Response::new(Box::pin(stream)))
    }
    async fn local_file_page(
        &self,
        r: Request<LocalFilePageRequest>,
    ) -> Result<Response<LocalFilePageReply>, Status> {
        let req = r.into_inner();

        let (file, next_page_token) = self
            .runtime
            .local_file_page(req.page_size, req.page_token, req.oldest_first)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = LocalFilePageReply {
            file,
            next_page_token,
        };

        Ok(Response::new(resp))
    }

    async fn random_local_file(
        &self,
        r: Request<RandomLocalFileRequest>,
    ) -> Result<Response<RandomLocalFileReply>, Status> {
        let req = r.into_inner();

        let file = self
            .runtime
            .random_local_file(req.count)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = RandomLocalFileReply { file };

        Ok(Response::new(resp))
    }

    async fn tags(
        &self,
        r: Request<TagsRequest>,
    ) -> Result<Response<TagsReply>, Status> {
        let runtime = &self.runtime;
        let req = r.into_inner();

        // Check that the CID is actually indexed before tagging it
        runtime.indexed_file(req.cid.clone()).await.map_err(|_| {
            Status::internal("CID is not indexed so it cannot be tagged")
        })?;

        let tags = runtime
            .tags(req.cid)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let reply = TagsReply { tags };

        Ok(Response::new(reply))
    }

    async fn forget_file(
        &self,
        _: Request<ForgetFileRequest>,
    ) -> Result<Response<ForgetFileReply>, Status> {
        let reply = ForgetFileReply {};

        Ok(Response::new(reply))
    }

    async fn cid_info(
        &self,
        r: Request<CidInfoRequest>,
    ) -> Result<Response<CidInfoReply>, Status> {
        let req = r.into_inner();
        let file =
            Some(self.runtime.indexed_file(req.cid).await.map_err(|_| {
                Status::internal("CID is not indexed so it cannot be tagged")
            })?);

        Ok(Response::new(CidInfoReply { file }))
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

    let default_db_uri = format!(
        "sqlite://{}",
        default_filestore_path
            .join("hooya.sqlite")
            .to_str()
            .unwrap()
    );

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
        .arg(Arg::new("db-uri").long("db-uri").env("HOOYAD_DB_URI"))
        .get_matches();

    let filestore_path = matches
        .get_one::<PathBuf>("filestore")
        .unwrap_or(&default_filestore_path);

    // Create filestore structure
    create_dir_all(filestore_path.join("store"))?;
    create_dir_all(filestore_path.join("forgotten"))?;
    create_dir_all(filestore_path.join("thumbs"))?;
    create_dir_all(filestore_path.join("tmp"))?;

    let db_uri = matches
        .get_one::<String>("db-uri")
        .unwrap_or(&default_db_uri);

    let mut should_init = false;
    // TODO Match on URI for different DB types
    if !Sqlite::database_exists(db_uri).await.unwrap_or(false) {
        Sqlite::create_database(db_uri).await?;
        should_init = true;
    }

    let mut db = hooya::local::Db::new(SqlitePool::connect(db_uri).await?);

    if should_init {
        db.init_tables().await?;
    }

    Server::builder()
        .add_service(ControlServer::new(IControl {
            runtime: Runtime {
                filestore_path: filestore_path.to_path_buf(),
                db,
            },
        }))
        .serve(matches.get_one::<String>("endpoint").unwrap().parse()?)
        .await?;
    Ok(())
}
