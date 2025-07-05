use anyhow::Result;
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post, put},
    Form, Json, Router,
};
use bcrypt::verify;
use clap::{command, value_parser, Arg, Command};
use dotenv::dotenv;
use futures_util::{StreamExt, TryStreamExt};
use hooya::proto::{
    control_client::ControlClient, AllFilesRequest, AllTagsRequest,
    CidInfoRequest, CidThumbnailRequest, CompleteUploadRequest,
    ContentAtCidRequest, GetUploadStatusRequest, LocalFilePageRequest,
    ProcessingEventsRequest, SearchQuery, SearchRequest,
    StartUploadSessionRequest, SuggestTagRequest, Tag, TagQuery, TagsRequest,
    Thumbnail, UploadChunkRequest,
};
use jsonwebtoken::{
    decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tonic::transport::Channel;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Clone)]
struct AState {
    client: ControlClient<Channel>,
    jwt_secret: [u8; 32],
    config: hooya_config::RuntimeConfig,
}

pub const DEFAULT_HOOYA_WEB_PROXY_ENDPOINT: &str = "0.0.0.0:8532";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let matches = command!()
        .subcommand_required(false)
        .arg_required_else_help(false)
        .subcommand(
            Command::new("set-password")
                .about("Set web UI password")
                .arg(
                    Arg::new("password")
                        .help("The password to set")
                        .required(true),
                ),
        )
        .arg(
            Arg::new("hooyad-endpoint")
                .long("endpoint")
                .env("HOOYAD_ENDPOINT")
                .default_value(hooya_config::DEFAULT_HOOYAD_ENDPOINT),
        )
        .arg(
            Arg::new("proxy-endpoint")
                .long("proxy-endpoint")
                .env("HOOYA_WEB_PROXY_ENDPOINT")
                .default_value(DEFAULT_HOOYA_WEB_PROXY_ENDPOINT),
        )
        .arg(
            Arg::new("data-dir")
                .long("data-dir")
                .env("HOOYA_DATADIR")
                .value_parser(value_parser!(PathBuf))
                .default_value(&**hooya_config::DEFAULT_DATA_DIR),
        )
        .arg(
            Arg::new("cors-origins")
                .long("cors-origins")
                .env("CORS_ORIGINS")
                .help("CORS origins: 'localhost' for any localhost port, or comma-separated URLs")
                .default_value("localhost"),
        )
        .get_matches();

    // data dir
    let data_dir = matches.get_one::<PathBuf>("data-dir").unwrap().clone();
    let config = hooya_config::RuntimeConfig::new(data_dir);

    if let Some(("set-password", sub_matches)) = matches.subcommand() {
        let password = sub_matches.get_one::<String>("password").unwrap();
        config.store_password_hash(password)?;
        println!("Password updated successfully");
        return Ok(());
    }

    let jwt_secret = config.ensure_jwt_secret_exists()?;
    let state = AState {
        client: ControlClient::connect(format!(
            "http://{}",
            matches.get_one::<String>("hooyad-endpoint").unwrap()
        ))
        .await?,
        jwt_secret,
        config: config.clone(),
    };

    // for ease of startup
    if let Some(new_password) = config.ensure_password_exists()? {
        println!("generated operator password: {}", new_password);
    }

    // configure CORS
    let cors_origins = matches.get_one::<String>("cors-origins").unwrap();
    let cors_layer = if cors_origins == "localhost" {
        // allow any localhost port
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(|origin, _request_parts| {
                let origin_str = origin.as_bytes();
                origin_str.starts_with(b"http://localhost:")
                    || origin_str.starts_with(b"https://localhost:")
                    || origin_str == b"http://localhost"
                    || origin_str == b"https://localhost"
            }))
            .allow_methods([Method::GET, Method::POST, Method::PUT])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
            ])
            .allow_credentials(true)
    } else {
        // if not localhost, parse origins
        let origins: Result<Vec<_>, _> = cors_origins
            .split(',')
            .map(|s| s.trim().parse::<axum::http::HeaderValue>())
            .collect();

        match origins {
            Ok(origins) => CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods([Method::GET, Method::POST, Method::PUT])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                ])
                .allow_credentials(true),
            Err(_) => {
                eprintln!("invalid CORS origins format: {}", cors_origins);
                std::process::exit(1);
            }
        }
    };

    let app = Router::new()
        .route("/cid-content/:cid", get(cid_content))
        .route("/cid-thumbnail/:cid/medium", get(cid_thumbnail_medium))
        .route("/cid-thumbnail/:cid/small", get(cid_thumbnail_small))
        .route("/cid-thumbnail/:cid/:long_edge", get(cid_thumbnail))
        .route("/cid-tags/:cid", get(cid_tags))
        .route("/cid-info/:cid", get(cid_info))
        .route("/local-file-page/:page_token", get(local_file_page))
        .route("/all-files/:page_token", get(all_files))
        .route("/all-tags/:page_token", get(all_tags))
        .route("/search-files/:query/:page_token", get(search_files))
        .route("/suggest-tag/:query", get(suggest_tag_with_query))
        .route("/suggest-tag", get(suggest_tag))
        .route("/login", post(login))
        .route("/tag-cid/:cid", post(tag_cid))
        .route("/start-upload", post(start_upload))
        .route("/upload-chunk/:upload_id/:chunk_index", put(upload_chunk))
        .route("/complete-upload/:upload_id", post(complete_upload))
        .route("/upload-status/:upload_id", get(upload_status))
        .route("/api/events/processing/:cid", get(processing_events))
        .layer(cors_layer)
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind::<String>(
        matches
            .get_one::<String>("proxy-endpoint")
            .unwrap()
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();

    axum::serve(listener, app).await.unwrap();

    Ok(())
}

#[derive(Deserialize)]
struct LoginData {
    password: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ClaimData {
    user_id: u64,
    exp: usize,
}

async fn login(
    State(state): State<AState>,
    Form(payload): Form<LoginData>,
) -> impl IntoResponse {
    // check if this is a token refresh request
    if let Some(refresh_token) = payload.refresh_token {
        match decode::<ClaimData>(
            &refresh_token,
            &DecodingKey::from_secret(&state.jwt_secret),
            &Validation::default(),
        ) {
            Ok(token_data) => {
                // check if token is expired
                if token_data.claims.exp
                    < chrono::Utc::now().timestamp() as usize
                {
                    return (StatusCode::UNAUTHORIZED, "Token expired")
                        .into_response();
                }

                // issue new token with same user_id
                let claims = ClaimData {
                    user_id: token_data.claims.user_id,
                    exp: chrono::Utc::now().timestamp() as usize + 3600, // 1hr
                };

                match encode(
                    &Header::default(),
                    &claims,
                    &EncodingKey::from_secret(&state.jwt_secret),
                ) {
                    Ok(token) => token.into_response(),
                    Err(_) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to generate token",
                    )
                        .into_response(),
                }
            }
            Err(_) => {
                (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            }
        }
    } else if let Some(password) = payload.password {
        // password login flow
        let password_hash = match state.config.load_password_hash() {
            Ok(hash) => hash,
            Err(_) => {
                return (
                    StatusCode::PRECONDITION_FAILED,
                    "Password not configured",
                )
                    .into_response()
            }
        };

        if verify(&password, &password_hash).unwrap_or(false) {
            let claims = ClaimData {
                user_id: 1,
                exp: chrono::Utc::now().timestamp() as usize + 3600, // 1hr
            };

            match encode(
                &Header::default(),
                &claims,
                &EncodingKey::from_secret(&state.jwt_secret),
            ) {
                Ok(token) => token.into_response(),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to generate token",
                )
                    .into_response(),
            }
        } else {
            (StatusCode::UNAUTHORIZED, "Invalid credentials").into_response()
        }
    } else {
        (
            StatusCode::BAD_REQUEST,
            "Password or refresh_token required",
        )
            .into_response()
    }
}

fn require_auth(
    state: &AState,
    headers: HeaderMap,
) -> Result<ClaimData, impl IntoResponse> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    let token = match auth_header {
        Some(token) => token,
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Authorization header missing or invalid",
            )
                .into_response())
        }
    };

    let token_data = match decode::<ClaimData>(
        token,
        &DecodingKey::from_secret(&state.jwt_secret),
        &Validation::new(Algorithm::HS256),
    ) {
        Ok(data) => data,
        Err(_) => {
            return Err(
                (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            )
        }
    };

    if token_data.claims.exp < chrono::Utc::now().timestamp() as usize {
        return Err((StatusCode::UNAUTHORIZED, "Expired token").into_response());
    }

    Ok(token_data.claims)
}

#[derive(Deserialize)]
struct TagCidData {
    tags: Vec<Tag>,
}

async fn tag_cid(
    State(mut state): State<AState>,
    headers: HeaderMap,
    Path(encoded_cid): Path<String>,
    Form(payload): Form<TagCidData>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, headers) {
        return e.into_response();
    }

    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let tags = payload.tags;
    state
        .client
        .tag_cid(hooya::proto::TagCidRequest { cid, tags })
        .await
        .unwrap();

    StatusCode::CREATED.into_response()
}

async fn cid_content(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let mut client = state.client;
    let chunk_stream = client
        .content_at_cid(ContentAtCidRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .into_stream()
        .and_then(|f| futures::future::ok(FileChunk(f)));
    let body = axum::body::Body::from_stream(chunk_stream);

    let file_info = client
        .cid_info(CidInfoRequest { cid })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let mut headers = HeaderMap::new();
    headers.append(
        axum::http::header::CACHE_CONTROL,
        "max-age=31536000, immutable".parse().unwrap(),
    );
    headers.append(axum::http::header::CONTENT_LENGTH, file_info.size.into());

    if let Some(mtype) = file_info.mimetype {
        let save_extension = mimetype_extension(&mtype);

        headers
            .append(axum::http::header::CONTENT_TYPE, mtype.parse().unwrap());

        if let Some(save_extension) = save_extension {
            headers.append(
                axum::http::header::CONTENT_DISPOSITION,
                format!(
                    "inline; filename=\"{}.{}\"",
                    encoded_cid, save_extension
                )
                .parse()
                .unwrap(),
            );
        }
    }

    (headers, body).into_response()
}

async fn cid_thumbnail_medium(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let mut client = state.client;

    let file_info = client
        .cid_info(CidInfoRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let thumbs = match extract_thumbnails(&file_info) {
        Ok(thumbs) => thumbs,
        Err(e) => return e.into_response(),
    };

    let thumbnail = closest_thumbnail(&thumbs, 1280);
    let long_edge = if thumbnail.width > thumbnail.height {
        thumbnail.width
    } else {
        thumbnail.height
    }
    .try_into()
    .unwrap();

    let chunk_stream =
        create_thumbnail_stream(&mut client, cid, long_edge).await;
    let body = axum::body::Body::from_stream(chunk_stream);
    let headers =
        create_thumbnail_headers(&thumbnail, &encoded_cid, Some(long_edge));

    (headers, body).into_response()
}

async fn cid_thumbnail_small(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let mut client = state.client;

    let file_info = client
        .cid_info(CidInfoRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let thumbs = match extract_thumbnails(&file_info) {
        Ok(thumbs) => thumbs,
        Err(e) => return e.into_response(),
    };

    let thumbnail = closest_thumbnail(&thumbs, 640);
    let long_edge = if thumbnail.width > thumbnail.height {
        thumbnail.width
    } else {
        thumbnail.height
    }
    .try_into()
    .unwrap();

    let chunk_stream =
        create_thumbnail_stream(&mut client, cid, long_edge).await;
    let body = axum::body::Body::from_stream(chunk_stream);
    let headers =
        create_thumbnail_headers(&thumbnail, &encoded_cid, Some(long_edge));

    (headers, body).into_response()
}

async fn cid_thumbnail(
    State(state): State<AState>,
    Path((encoded_cid, long_edge)): Path<(String, u32)>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let mut client = state.client;

    let file_info = client
        .cid_info(CidInfoRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let thumbs = match extract_thumbnails(&file_info) {
        Ok(thumbs) => thumbs,
        Err(e) => return e.into_response(),
    };

    let thumb_match = thumbs.iter().find(|t| {
        (t.width == long_edge as i64 && t.height < t.width)
            || (t.height == long_edge as i64 && t.height > t.width)
    });

    let thumb = match thumb_match {
        Some(t) => t,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                "No such sized thumbnail for this indexed CID",
            )
                .into_response()
        }
    };

    let chunk_stream =
        create_thumbnail_stream(&mut client, cid, long_edge).await;
    let body = axum::body::Body::from_stream(chunk_stream);
    let headers =
        create_thumbnail_headers(&thumb, &encoded_cid, Some(long_edge));

    (headers, body).into_response()
}

fn closest_thumbnail(thumbnails: &[Thumbnail], long_edge: i64) -> &Thumbnail {
    thumbnails
        .iter()
        .max_by(|x, y| {
            if x.width > x.height {
                ((y.width - long_edge).abs()).cmp(&(x.width - long_edge).abs())
            } else {
                ((y.height - long_edge).abs())
                    .cmp(&(x.height - long_edge).abs())
            }
        })
        .unwrap()
}

fn mimetype_extension(mimetype: &str) -> Option<String> {
    match mimetype {
        "image/jpeg" => Some("jpeg".to_string()),
        "image/png" => Some("png".to_string()),
        "image/gif" => Some("gif".to_string()),
        "video/mp4" => Some("mp4".to_string()),
        _ => None,
    }
}

// helper functions for common api patterns

fn decode_cid_param(encoded_cid: &str) -> Result<Vec<u8>, impl IntoResponse> {
    match hooya::cid::decode(encoded_cid) {
        Ok((_, cid)) => Ok(cid),
        Err(_) => Err((StatusCode::BAD_REQUEST, "Invalid CID").into_response()),
    }
}

fn file_info_to_response(
    info: hooya::proto::File,
) -> proxy_response::CidInfoResponse {
    let cid = hooya::cid::encode(info.cid);
    let size = info.size;
    let mimetype = info.mimetype;
    let ext_file = info.ext_file.map(|f| f.into());
    let processing_status = info.processing_status;

    proxy_response::CidInfoResponse {
        cid,
        size,
        mimetype,
        ext_file,
        processing_status,
    }
}

fn extract_thumbnails(
    file_info: &hooya::proto::File,
) -> Result<Vec<hooya::proto::Thumbnail>, impl IntoResponse> {
    let ext_file = match &file_info.ext_file {
        Some(ext_file) => ext_file,
        None => {
            return Err(
                (StatusCode::NOT_FOUND, "No such CID indexed").into_response()
            )
        }
    };

    let thumbs = match ext_file {
        hooya::proto::file::ExtFile::Image(i) => &i.thumbnails,
        hooya::proto::file::ExtFile::Video(v) => &v.thumbnails,
    };

    Ok(thumbs.clone())
}

fn create_thumbnail_headers(
    thumbnail: &hooya::proto::Thumbnail,
    encoded_cid: &str,
    long_edge: Option<u32>,
) -> HeaderMap {
    let mut headers = HeaderMap::new();

    headers.append(
        axum::http::header::CACHE_CONTROL,
        "max-age=31536000, immutable".parse().unwrap(),
    );
    headers.append(axum::http::header::CONTENT_LENGTH, thumbnail.size.into());
    headers.append(
        axum::http::header::CONTENT_TYPE,
        thumbnail.mimetype.parse().unwrap(),
    );

    let save_extension = mimetype_extension(&thumbnail.mimetype);
    let filename = match (save_extension, long_edge) {
        (Some(ext), Some(edge)) => {
            format!("{}_thumb{}.{}", encoded_cid, edge, ext)
        }
        (Some(ext), None) => format!("{}_thumb.{}", encoded_cid, ext),
        (None, Some(edge)) => format!("{}_thumb{}", encoded_cid, edge),
        (None, None) => format!("{}_thumb", encoded_cid),
    };

    headers.append(
        axum::http::header::CONTENT_DISPOSITION,
        format!("inline; filename=\"{}\"", filename)
            .parse()
            .unwrap(),
    );

    headers
}

async fn create_thumbnail_stream(
    client: &mut ControlClient<Channel>,
    cid: Vec<u8>,
    long_edge: u32,
) -> impl futures_util::Stream<Item = Result<FileChunk, tonic::Status>> {
    client
        .cid_thumbnail(CidThumbnailRequest {
            source_cid: cid,
            long_edge,
        })
        .await
        .unwrap()
        .into_inner()
        .and_then(|f| futures::future::ok(FileChunk(f)))
}

async fn cid_tags(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let mut client = state.client;

    let tags: Vec<Tag> = client
        .tags(TagsRequest { cid })
        .await
        .unwrap()
        .into_inner()
        .tags;

    axum::Json(tags).into_response()
}

async fn cid_info(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    let mut client = state.client;

    let info: Option<hooya::proto::File> = client
        .cid_info(CidInfoRequest { cid })
        .await
        .unwrap()
        .into_inner()
        .file;

    let info = match info {
        Some(i) => i,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "No Info")
                .into_response()
        }
    };

    let body = file_info_to_response(info);

    axum::Json(body).into_response()
}

async fn local_file_page(
    State(state): State<AState>,
    Path(page_token): Path<String>,
) -> impl IntoResponse {
    let mut client = state.client;

    let local_file_page_resp = client
        .local_file_page(LocalFilePageRequest {
            oldest_first: false,
            page_token,
            page_size: 50,
        })
        .await
        .unwrap()
        .into_inner();

    let next_page_token = local_file_page_resp.next_page_token;
    let cid = local_file_page_resp
        .file
        .iter()
        .map(|f| hooya::cid::encode(f.cid.clone()))
        .collect();

    let body = proxy_response::LocalFilePageResponse {
        cid,
        next_page_token,
    };

    axum::Json(body).into_response()
}

async fn all_files(
    State(state): State<AState>,
    Path(page_token): Path<String>,
) -> impl IntoResponse {
    let mut client = state.client;

    let all_files_resp = client
        .all_files(AllFilesRequest {
            sort_order: 0,
            reverse_order: false,
            page_token,
            page_size: 50,
        })
        .await
        .unwrap()
        .into_inner();

    let final_page_token = all_files_resp.final_page_token;
    let next_page_token = all_files_resp.next_page_token;
    let files = all_files_resp
        .files
        .into_iter()
        .map(file_info_to_response)
        .collect();

    let body = proxy_response::AllFilesResponse {
        files,
        next_page_token,
        final_page_token,
    };

    axum::Json(body).into_response()
}

async fn all_tags(
    State(state): State<AState>,
    Path(page_token): Path<String>,
) -> impl IntoResponse {
    let mut client = state.client;

    let all_tags_resp = client
        .all_tags(AllTagsRequest {
            sort_order: 0,
            reverse_order: false,
            page_token,
            page_size: 50,
        })
        .await
        .unwrap()
        .into_inner();

    let next_page_token = all_tags_resp.next_page_token;
    let final_page_token = all_tags_resp.final_page_token;
    let tags = all_tags_resp
        .tags
        .into_iter()
        .map(|info| {
            let namespace = info.namespace;
            let descriptor = info.descriptor;
            let count = info.count;

            proxy_response::TagInfo {
                namespace,
                descriptor,
                count,
            }
        })
        .collect();

    let body = proxy_response::AllTagsResponse {
        tags,
        next_page_token,
        final_page_token,
    };

    axum::Json(body).into_response()
}

async fn suggest_tag(State(state): State<AState>) -> impl IntoResponse {
    let mut client = state.client;

    let query = SuggestTagRequest {
        tag_query: vec![],
        suggest_string: "".to_string(),
        max_suggest: 10,
    };

    let resp = client.suggest_tag(query).await.unwrap().into_inner();

    axum::Json(resp).into_response()
}
async fn suggest_tag_with_query(
    State(state): State<AState>,
    Path(query): Path<String>,
) -> impl IntoResponse {
    let mut client = state.client;

    let tags: Vec<&str> = query.split(',').collect();

    let existing_tags = if tags.len() > 1 {
        tags[..tags.len() - 1].to_vec()
    } else {
        vec![]
    };

    let existing_tags = existing_tags
        .iter()
        .map(|t| match t.split_once(':') {
            Some((namespace, descriptor)) => TagQuery {
                namespace: Some(namespace.to_string()),
                descriptor: descriptor.to_string(),
                negated: false,
            },
            None => TagQuery {
                namespace: None,
                descriptor: t.to_string(),
                negated: false,
            },
        })
        .collect();

    // Will always have at least one element because request was routed here
    let suggest_string = tags[tags.len() - 1].to_string();

    let query = SuggestTagRequest {
        tag_query: existing_tags,
        suggest_string,
        max_suggest: 10,
    };

    let resp = client.suggest_tag(query).await.unwrap().into_inner();

    let tag_constraints = resp.tag_constraints;
    let tag_suggestion = resp
        .tag_suggestion
        .into_iter()
        .filter(|t| {
            // When constraints are added, filter those constraints from the
            // result set. This could probably be handled somewhere else but
            // we may not always want this behavior in the future
            for c in &tag_constraints {
                if c.namespace == t.namespace && c.descriptor == t.descriptor {
                    return false;
                }
            }
            true
        })
        .collect();

    axum::Json(hooya::proto::SuggestTagReply {
        tag_constraints,
        tag_suggestion,
    })
    .into_response()
}

async fn search_files(
    State(state): State<AState>,
    Path((query, page_token)): Path<(String, String)>,
) -> impl IntoResponse {
    let mut client = state.client;

    let tag_query = query
        .split(',')
        .map(|t| {
            if let Some((namespace, descriptor)) = t.split_once(':') {
                (Some(namespace.to_string()), descriptor.to_string())
            } else {
                (None, t.to_string())
            }
        })
        .map(|(namespace, descriptor)| TagQuery {
            namespace,
            descriptor,
            negated: false,
        })
        .collect();

    let search_query = SearchQuery {
        tag_query,
        // None of the cool filters used here; simply namespace:descriptor queries
        size: None,
        mimetype: None,
        ext_attr: None,
    };

    let search_files_resp = client
        .search(SearchRequest {
            search_query: Some(search_query),
            sort_order: 0,
            reverse_order: false,
            page_token,
            page_size: 50,
        })
        .await
        .unwrap()
        .into_inner();

    let final_page_token = search_files_resp.final_page_token;
    let next_page_token = search_files_resp.next_page_token;
    let files = search_files_resp
        .files
        .into_iter()
        .map(file_info_to_response)
        .collect();

    let body = proxy_response::AllFilesResponse {
        files,
        next_page_token,
        final_page_token,
    };

    axum::Json(body).into_response()
}

#[derive(Deserialize)]
struct StartUploadRequest {
    size: Option<u64>,
    mimetype: Option<String>,
    chunk_size: u32,
}

#[derive(Serialize)]
struct StartUploadResponse {
    upload_id: String,
    chunk_size: u32,
}

async fn start_upload(
    State(mut state): State<AState>,
    headers: HeaderMap,
    Json(payload): Json<StartUploadRequest>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, headers) {
        return e.into_response();
    }

    let request = StartUploadSessionRequest {
        size: payload.size,
        mimetype: payload.mimetype,
        chunk_size: payload.chunk_size,
    };

    match state.client.start_upload_session(request).await {
        Ok(response) => {
            let reply = response.into_inner();
            Json(StartUploadResponse {
                upload_id: reply.upload_id,
                chunk_size: reply.chunk_size,
            })
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to start upload: {}", e),
        )
            .into_response(),
    }
}

async fn upload_chunk(
    State(mut state): State<AState>,
    headers: HeaderMap,
    Path((upload_id, chunk_index)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, headers) {
        return e.into_response();
    }

    let request = UploadChunkRequest {
        upload_id,
        chunk_index,
        data: body.to_vec(),
    };

    match state.client.upload_chunk(request).await {
        Ok(response) => {
            let reply = response.into_inner();
            let body = proxy_response::UploadChunkResponse {
                status: reply.status,
                bytes_received: reply.bytes_received,
                next_chunk_index: reply.next_chunk_index,
                error_message: reply.error_message,
            };
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to upload chunk: {}", e),
        )
            .into_response(),
    }
}

async fn complete_upload(
    State(mut state): State<AState>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, headers) {
        return e.into_response();
    }

    let request = CompleteUploadRequest { upload_id };

    match state.client.complete_upload(request).await {
        Ok(response) => {
            let reply = response.into_inner();
            let cid_bytes = reply.cid.clone();
            let file_info = reply.file.clone();

            let body = proxy_response::CompleteUploadResponse {
                cid: hooya::cid::encode(cid_bytes),
                file: file_info,
            };
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to complete upload: {}", e),
        )
            .into_response(),
    }
}

async fn upload_status(
    State(mut state): State<AState>,
    headers: HeaderMap,
    Path(upload_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, headers) {
        return e.into_response();
    }

    let request = GetUploadStatusRequest { upload_id };

    match state.client.get_upload_status(request).await {
        Ok(response) => {
            let reply = response.into_inner();
            let body = proxy_response::UploadStatusResponse {
                status: reply.status,
                bytes_received: reply.bytes_received,
                expected_size: reply.expected_size,
                next_chunk_index: reply.next_chunk_index,
                error_message: reply.error_message,
            };
            Json(body).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get upload status: {}", e),
        )
            .into_response(),
    }
}

async fn processing_events(
    Path(encoded_cid): Path<String>,
    State(state): State<AState>,
) -> impl IntoResponse {
    let cid = match decode_cid_param(&encoded_cid) {
        Ok(cid) => cid,
        Err(e) => return e.into_response(),
    };

    // start grpc stream
    let request = ProcessingEventsRequest { cid: cid.clone() };

    let mut stream = match state.client.clone().processing_events(request).await
    {
        Ok(response) => response.into_inner(),
        Err(_) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to start processing stream",
            )
                .into_response()
        }
    };

    // convert grpc stream to sse stream
    let sse_stream = async_stream::stream! {

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(event) => {
                    let event_name = match event.event_type {
                        0 => "processing_finished",      // ProcessingStatus::ProcessingFinished = 0
                        1 => "processing_started",       // ProcessingStatus::ProcessingStarted = 1
                        2 => "processing_failed",        // ProcessingStatus::ProcessingFailed = 2
                        3 => "thumbnail_generated",      // ProcessingStatus::ThumbnailGenerated = 3
                        4 => "video_preview_generated",  // ProcessingStatus::VideoPreviewGenerated = 4
                        _ => "unknown",
                    };

                    // include metadata for thumbnail/video events
                    let event_data = match event.event_type {
                        3 | 4 => { // thumbnail_generated or video_preview_generated
                            format!("{{\"long_edge\":{},\"mimetype\":\"{}\"}}",
                                event.long_edge.unwrap_or(0),
                                event.mimetype.as_deref().unwrap_or("")
                            )
                        }
                        _ => "{}".to_string()
                    };

                    let sse_event = Event::default()
                        .event(event_name)
                        .data(event_data);
                    yield Ok(sse_event);
                }
                Err(e) => {
                    yield Err(anyhow::anyhow!("grpc error: {}", e));
                    break;
                }
            }
        }
    };

    Sse::new(sse_stream).into_response()
}

mod proxy_response {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct LocalFilePageResponse {
        pub cid: Vec<String>,
        pub next_page_token: String,
    }

    #[derive(Serialize, Deserialize)]
    pub struct AllFilesResponse {
        pub files: Vec<CidInfoResponse>,
        pub next_page_token: String,
        pub final_page_token: String,
    }

    #[derive(Serialize, Deserialize)]
    pub struct AllTagsResponse {
        pub tags: Vec<TagInfo>,
        pub next_page_token: String,
        pub final_page_token: String,
    }

    #[derive(Serialize, Deserialize)]
    pub struct CidInfoResponse {
        pub cid: String,
        pub size: i64,
        pub mimetype: Option<String>,
        pub ext_file: Option<ExtFile>,
        pub processing_status: i32,
    }

    #[derive(Serialize, Deserialize)]
    pub struct TagInfo {
        pub namespace: String,
        pub descriptor: String,
        pub count: i64,
    }

    #[derive(Serialize, Deserialize)]
    pub struct Image {
        pub height: i64,
        pub width: i64,
        pub aspect_ratio: f32,
        pub colors: Vec<Vec<u8>>,
        pub thumbnails: Vec<Thumbnail>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct Video {
        pub height: i64,
        pub width: i64,
        pub aspect_ratio: f32,
        pub duration: f32,
        pub thumbnails: Vec<Thumbnail>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct Thumbnail {
        pub cid: String,
        pub source_cid: String,
        pub size: i64,
        pub height: i64,
        pub mimetype: String,
        pub width: i64,
        pub aspect_ratio: f32,
        pub is_animated: bool,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum ExtFile {
        Image(Image),
        Video(Video),
    }

    impl From<hooya::proto::file::ExtFile> for ExtFile {
        fn from(e: hooya::proto::file::ExtFile) -> Self {
            match e {
                hooya::proto::file::ExtFile::Image(i) => {
                    ExtFile::Image(Image {
                        height: i.height,
                        width: i.width,
                        aspect_ratio: i.aspect_ratio,
                        colors: i.colors,
                        thumbnails: i
                            .thumbnails
                            .into_iter()
                            .map(|t| t.into())
                            .collect(),
                    })
                }
                hooya::proto::file::ExtFile::Video(v) => {
                    ExtFile::Video(Video {
                        height: v.height,
                        width: v.width,
                        aspect_ratio: v.aspect_ratio,
                        duration: v.duration,
                        thumbnails: v
                            .thumbnails
                            .into_iter()
                            .map(|t| t.into())
                            .collect(),
                    })
                }
            }
        }
    }

    impl From<hooya::proto::Thumbnail> for Thumbnail {
        fn from(t: hooya::proto::Thumbnail) -> Self {
            Thumbnail {
                cid: hooya::cid::encode(t.cid),
                source_cid: hooya::cid::encode(t.source_cid),
                mimetype: t.mimetype,
                size: t.size,
                height: t.height,
                width: t.width,
                aspect_ratio: t.aspect_ratio,
                is_animated: t.is_animated,
            }
        }
    }

    #[derive(Serialize, Deserialize)]
    pub struct UploadChunkResponse {
        pub status: i32,
        pub bytes_received: u64,
        pub next_chunk_index: String,
        pub error_message: Option<String>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct CompleteUploadResponse {
        pub cid: String,
        pub file: Option<hooya::proto::File>,
    }

    #[derive(Serialize, Deserialize)]
    pub struct UploadStatusResponse {
        pub status: i32,
        pub bytes_received: u64,
        pub expected_size: u64,
        pub next_chunk_index: String,
        pub error_message: Option<String>,
    }
}

struct FileChunk(hooya::proto::FileChunk);

impl From<FileChunk> for axum::body::Bytes {
    fn from(value: FileChunk) -> Self {
        value.0.data.into()
    }
}
