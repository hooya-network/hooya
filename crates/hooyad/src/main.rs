use clap::{command, value_parser, Arg};
use dotenv::dotenv;
use futures_util::Stream;
use hooya::proto::{
    control_server::{Control, ControlServer},
    AllFilesReply, AllFilesRequest, AllTagsReply, AllTagsRequest, CidInfoReply,
    CidInfoRequest, CidThumbnailRequest, CompleteUploadReply,
    CompleteUploadRequest, ContentAtCidRequest, FileChunk, ForgetFileReply,
    ForgetFileRequest, GetUploadStatusReply, GetUploadStatusRequest,
    LocalFilePageReply, LocalFilePageRequest, ProcessingEvent,
    ProcessingEventsRequest, ProcessingStatus, RandomLocalFileReply,
    RandomLocalFileRequest, ReimportReply, ReimportRequest, SearchReply,
    SearchRequest, StartUploadSessionReply, StartUploadSessionRequest,
    StreamToFilestoreReply, SuggestTagReply, SuggestTagRequest, TagCidReply,
    TagCidRequest, TagsReply, TagsRequest, UploadChunkReply,
    UploadChunkRequest, UploadStatus, VersionReply, VersionRequest,
};
use hooya::runtime::Runtime;
use rand::distributions::DistString;
use sqlx::migrate::MigrateDatabase;
use sqlx::{Sqlite, SqlitePool};
use std::{
    collections::HashMap, fs::File, io::Write, path::PathBuf, pin::Pin,
    sync::Arc, time::Instant,
};
use tokio::sync::Mutex;
use tokio_stream::StreamExt;
use tonic::{transport::Server, Request, Response, Status};

const MAX_CHUNK_SIZE: u32 = 10 * 1024 * 1024; // 10MB max chunk size

struct UploadSession {
    id: String,
    temp_file: File,
    expected_size: Option<u64>,
    mimetype: Option<String>,
    bytes_received: u64,
    next_expected_chunk: String,
    chunk_size: u32,
    started_at: Instant,
}

struct IControl {
    pub runtime: Arc<Runtime>,
    pub upload_sessions: Arc<Mutex<HashMap<String, UploadSession>>>,
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
            .import_from_filestore(cid.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let reply = StreamToFilestoreReply { cid };
        Ok(Response::new(reply))
    }

    async fn reimport(
        &self,
        r: Request<ReimportRequest>,
    ) -> Result<Response<ReimportReply>, Status> {
        let cid = r.into_inner().cid;
        self.runtime
            .import_from_filestore(cid.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let reply = ReimportReply {};
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

    async fn untag_cid(
        &self,
        r: Request<TagCidRequest>,
    ) -> Result<Response<TagCidReply>, Status> {
        let runtime = &self.runtime;
        let req = r.into_inner();

        let reply = TagCidReply {};

        // Check that the CID is actually indexed before untagging it
        runtime.indexed_file(req.cid.clone()).await.map_err(|_| {
            Status::invalid_argument(
                "CID is not indexed so it cannot be untagged",
            )
        })?;

        runtime
            .untag_cid(req.cid, req.tags)
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

    async fn all_files(
        &self,
        r: Request<AllFilesRequest>,
    ) -> Result<Response<AllFilesReply>, Status> {
        let req = r.into_inner();

        let (files, next_page_token, final_page_token) = self
            .runtime
            .all_files_page(
                req.page_size,
                req.page_token,
                req.sort_order,
                req.reverse_order,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = AllFilesReply {
            files,
            final_page_token: final_page_token.to_string(),
            next_page_token,
        };

        Ok(Response::new(resp))
    }

    async fn all_tags(
        &self,
        r: Request<AllTagsRequest>,
    ) -> Result<Response<AllTagsReply>, Status> {
        let req = r.into_inner();

        let (tags, next_page_token, final_page_token) = self
            .runtime
            .all_tags_page(
                req.page_size,
                req.page_token,
                req.sort_order,
                req.reverse_order,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = AllTagsReply {
            tags,
            final_page_token: final_page_token.to_string(),
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
            Status::internal("CID is not indexed so it has no tags")
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
                Status::internal("CID is not indexed so it has no info")
            })?);

        Ok(Response::new(CidInfoReply { file }))
    }

    async fn search(
        &self,
        r: Request<SearchRequest>,
    ) -> Result<Response<SearchReply>, Status> {
        let req = r.into_inner();
        let search_query = req
            .search_query
            .ok_or_else(|| Status::invalid_argument("No query specified"))?;

        let (files, next_page_token, final_page_token) = self
            .runtime
            .search_page(
                search_query,
                req.page_size,
                req.page_token,
                req.sort_order,
                req.reverse_order,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let resp = SearchReply {
            files,
            final_page_token: final_page_token.to_string(),
            next_page_token,
        };

        Ok(Response::new(resp))
    }

    async fn suggest_tag(
        &self,
        r: Request<SuggestTagRequest>,
    ) -> Result<Response<SuggestTagReply>, Status> {
        let req = r.into_inner();
        let existing_tags = req.tag_query;
        let suggest_string = req.suggest_string;

        let tag_suggestion = match suggest_string.split_once(':') {
            Some((namespace, incomplete_descriptor)) => self
                .runtime
                .suggest_tags_within_namespace(
                    &existing_tags,
                    namespace.to_string(),
                    incomplete_descriptor.to_string(),
                )
                .await
                .map_err(|e| Status::internal(e.to_string()))?,
            None => {
                if !existing_tags.is_empty() || !suggest_string.is_empty() {
                    self.runtime
                        .suggest_tags_without_namespace(
                            &existing_tags,
                            &suggest_string,
                        )
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?
                } else {
                    self.runtime
                        .suggest_all_tags()
                        .await
                        .map_err(|e| Status::internal(e.to_string()))?
                }
            }
        };

        let resp = SuggestTagReply {
            tag_constraints: existing_tags,
            tag_suggestion,
        };

        Ok(Response::new(resp))
    }

    async fn start_upload_session(
        &self,
        r: Request<StartUploadSessionRequest>,
    ) -> Result<Response<StartUploadSessionReply>, Status> {
        let req = r.into_inner();

        // random id
        let upload_id = rand::distributions::Alphanumeric
            .sample_string(&mut rand::thread_rng(), 16);

        // put a ceiling on chunk size
        let chunk_size = std::cmp::min(req.chunk_size, MAX_CHUNK_SIZE);

        // temp file for upload sessions
        let tmp_name = format!("{}_session", upload_id);
        let tmp_path = self.runtime.filestore_path.join("tmp").join(&tmp_name);
        let temp_file = File::create(&tmp_path).map_err(|e| {
            Status::internal(format!("Failed to create temp file: {}", e))
        })?;

        let session = UploadSession {
            id: upload_id.clone(),
            temp_file,
            expected_size: req.size,
            mimetype: req.mimetype,
            bytes_received: 0,
            next_expected_chunk: 0.to_string(),
            chunk_size,
            started_at: Instant::now(),
        };

        // Store session
        self.upload_sessions
            .lock()
            .await
            .insert(upload_id.clone(), session);

        let reply = StartUploadSessionReply {
            upload_id,
            chunk_size,
        };

        Ok(Response::new(reply))
    }

    async fn upload_chunk(
        &self,
        r: Request<UploadChunkRequest>,
    ) -> Result<Response<UploadChunkReply>, Status> {
        let req = r.into_inner();
        let mut sessions = self.upload_sessions.lock().await;

        let session = sessions
            .get_mut(&req.upload_id)
            .ok_or_else(|| Status::not_found("upload session not found"))?;

        // this ensures we don't append out of order if one was dropped
        if req.chunk_index != session.next_expected_chunk {
            return Ok(Response::new(UploadChunkReply {
                status: UploadStatus::UploadError.into(),
                error_message: Some(format!(
                    "expected chunk {}, got chunk {}",
                    session.next_expected_chunk, req.chunk_index
                )),
                bytes_received: session.bytes_received,
                next_chunk_index: session.next_expected_chunk.clone(),
            }));
        }

        // enforce that len is respected unless this is chunk would be the final one
        let this_data_len = req.data.len().try_into().unwrap_or(u64::MAX);
        if this_data_len != session.chunk_size as u64
            && session.bytes_received + this_data_len
                != session.expected_size.unwrap_or(u64::MAX)
        {
            return Ok(Response::new(UploadChunkReply {
                status: UploadStatus::UploadError.into(),
                error_message: Some(format!(
                    "chunk is not correct size: expected {} got {}",
                    session.chunk_size, this_data_len
                )),
                bytes_received: session.bytes_received,
                next_chunk_index: session.next_expected_chunk.clone(),
            }));
        }

        // write chunk data
        session.temp_file.write_all(&req.data).map_err(|e| {
            Status::internal(format!("failed to write chunk: {}", e))
        })?;

        session.bytes_received += this_data_len;
        let this_chunk_number = session
            .next_expected_chunk
            .parse::<u64>()
            .expect("could not parse chunk id as number");
        session.next_expected_chunk = (this_chunk_number + 1).to_string();

        // check if upload is complete
        let status = if let Some(expected_size) = session.expected_size {
            if session.bytes_received >= expected_size {
                UploadStatus::UploadComplete as i32
            } else {
                UploadStatus::UploadInProgress as i32
            }
        } else {
            UploadStatus::UploadInProgress as i32
        };

        let reply = UploadChunkReply {
            status,
            error_message: None,
            bytes_received: session.bytes_received,
            next_chunk_index: session.next_expected_chunk.clone(),
        };

        Ok(Response::new(reply))
    }

    async fn complete_upload(
        &self,
        r: Request<CompleteUploadRequest>,
    ) -> Result<Response<CompleteUploadReply>, Status> {
        let req = r.into_inner();
        let mut sessions = self.upload_sessions.lock().await;

        let session = sessions
            .remove(&req.upload_id)
            .ok_or_else(|| Status::not_found("upload session not found"))?;

        // Close temp file and reconstruct its path
        drop(session.temp_file);
        let tmp_name = format!("{}_session", session.id);
        let tmp_path = self.runtime.filestore_path.join("tmp").join(&tmp_name);

        let file_data = std::fs::read(&tmp_path).map_err(|e| {
            Status::internal(format!("failed to read tmp file: {}", e))
        })?;

        if file_data.is_empty() {
            return Err(Status::invalid_argument("empty file"));
        }

        // compute cid
        let mut sha_context = hooya::cid::new_digest_context();
        sha_context.update(&file_data);
        let cid = hooya::cid::wrap_digest(sha_context.finish())
            .map_err(|e| Status::internal(e.to_string()))?;

        // move to final location
        let cid_store_path = self
            .runtime
            .derive_store_path(&cid)
            .map_err(|e| Status::internal(e.to_string()))?;

        let parent = cid_store_path.parent().unwrap();
        if !parent.is_dir() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Status::internal(format!("failed to create directory: {}", e))
            })?;
        }

        std::fs::rename(&tmp_path, &cid_store_path).map_err(|e| {
            Status::internal(format!("failed to move file: {}", e))
        })?;

        // import once uploaded
        let file = self
            .runtime
            .import_basic_file_record(cid.clone())
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        // Spawn background processing for thumbnails
        let runtime_clone = Arc::clone(&self.runtime);
        let cid_clone = cid.clone();
        tokio::spawn(async move {
            runtime_clone.emit_processing_event(
                cid_clone.clone(),
                hooya::runtime::ProcessingEventType::Started,
                None,
            );

            match runtime_clone
                .process_file_background(cid_clone.clone())
                .await
            {
                Ok(_) => {
                    runtime_clone.emit_processing_event(
                        cid_clone,
                        hooya::runtime::ProcessingEventType::Finished,
                        None,
                    );
                }
                Err(e) => {
                    runtime_clone.emit_processing_event(
                        cid_clone,
                        hooya::runtime::ProcessingEventType::Failed,
                        Some(e.to_string()),
                    );
                }
            }
        });

        let reply = CompleteUploadReply {
            cid,
            file: Some(file),
        };

        Ok(Response::new(reply))
    }

    async fn get_upload_status(
        &self,
        r: Request<GetUploadStatusRequest>,
    ) -> Result<Response<GetUploadStatusReply>, Status> {
        let req = r.into_inner();
        let sessions = self.upload_sessions.lock().await;

        let session = sessions
            .get(&req.upload_id)
            .ok_or_else(|| Status::not_found("upload session not found"))?;

        let status = if let Some(expected_size) = session.expected_size {
            if session.bytes_received >= expected_size {
                UploadStatus::UploadComplete
            } else {
                UploadStatus::UploadInProgress
            }
        } else {
            UploadStatus::UploadInProgress
        } as i32;

        let reply = GetUploadStatusReply {
            status,
            bytes_received: session.bytes_received,
            expected_size: session.expected_size.unwrap_or(0),
            next_chunk_index: session.next_expected_chunk.clone(),
            error_message: None,
        };

        Ok(Response::new(reply))
    }

    type ProcessingEventsStream =
        Pin<Box<dyn Stream<Item = Result<ProcessingEvent, Status>> + Send>>;

    async fn processing_events(
        &self,
        request: Request<ProcessingEventsRequest>,
    ) -> Result<Response<Self::ProcessingEventsStream>, Status> {
        let req = request.into_inner();
        let target_cid = req.cid;

        let rx = self.runtime.processing_events.subscribe();

        use tokio_stream::wrappers::BroadcastStream;
        let stream = BroadcastStream::new(rx)
            .filter_map(move |result| {
                match result {
                    Ok(event) => {
                        if event.cid == target_cid {
                            let proto_event = ProcessingEvent {
                                cid: event.cid.clone(),
                                event_type: match &event.event_type {
                                    hooya::runtime::ProcessingEventType::Started => ProcessingStatus::ProcessingStarted as i32,
                                    hooya::runtime::ProcessingEventType::ThumbnailGenerated { .. } => ProcessingStatus::ThumbnailGenerated as i32,
                                    hooya::runtime::ProcessingEventType::VideoPreviewGenerated { .. } => ProcessingStatus::VideoPreviewGenerated as i32,
                                    hooya::runtime::ProcessingEventType::Finished => ProcessingStatus::ProcessingFinished as i32,
                                    hooya::runtime::ProcessingEventType::Failed => ProcessingStatus::ProcessingFailed as i32,
                                },
                                error_message: event.error_message.clone(),
                                long_edge: match &event.event_type {
                                    hooya::runtime::ProcessingEventType::ThumbnailGenerated { long_edge, .. } => Some(*long_edge),
                                    hooya::runtime::ProcessingEventType::VideoPreviewGenerated { long_edge, .. } => Some(*long_edge),
                                    _ => None,
                                },
                                mimetype: match &event.event_type {
                                    hooya::runtime::ProcessingEventType::ThumbnailGenerated { mimetype, .. } => Some(mimetype.clone()),
                                    hooya::runtime::ProcessingEventType::VideoPreviewGenerated { mimetype, .. } => Some(mimetype.clone()),
                                    _ => None,
                                },
                            };
                            Some(Ok(proto_event))
                        } else {
                            None
                        }
                    }
                    Err(_) => None
                }
            });

        Ok(Response::new(Box::pin(stream)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let matches = command!()
        .arg(
            Arg::new("endpoint")
                .long("endpoint")
                .env("HOOYAD_ENDPOINT")
                .default_value(hooya_config::DEFAULT_HOOYAD_ENDPOINT),
        )
        .arg(
            Arg::new("filestore")
                .long("filestore")
                .env("HOOYAD_FILESTORE")
                .value_parser(value_parser!(PathBuf))
                .default_value(&**hooya_config::DEFAULT_DATA_DIR),
        )
        .arg(Arg::new("db-uri").long("db-uri").env("HOOYAD_DB_URI"))
        .get_matches();

    // filestore path
    let filestore_path =
        matches.get_one::<PathBuf>("filestore").unwrap().clone();

    let config = hooya_config::RuntimeConfig::new(filestore_path.clone());
    let default_db_uri = config.sqlite_uri();

    // Create filestore structure
    config.ensure_filestore_structure()?;

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

    let (processing_events, _) = tokio::sync::broadcast::channel(1000);

    Server::builder()
        .accept_http1(true)
        .add_service(ControlServer::new(IControl {
            runtime: Arc::new(Runtime {
                filestore_path,
                db,
                processing_events,
                processing_cids: std::sync::Arc::new(tokio::sync::RwLock::new(
                    std::collections::HashSet::new(),
                )),
            }),
            upload_sessions: Arc::new(Mutex::new(HashMap::new())),
        }))
        .serve(matches.get_one::<String>("endpoint").unwrap().parse()?)
        .await?;
    Ok(())
}
