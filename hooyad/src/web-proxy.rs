use anyhow::Result;
use serde::{Serialize, Deserialize};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::get,
    Router,
};
use clap::{command, Arg};
use dotenv::dotenv;
use hooya::proto::{
    control_client::ControlClient, CidInfoRequest, CidThumbnailRequest,
    ContentAtCidRequest, Thumbnail, Tag, TagsRequest, LocalFilePageRequest,
};
use tonic::transport::Channel;
mod config;

#[derive(Clone)]
struct AState {
    client: ControlClient<Channel>,
}

pub const DEFAULT_HOOYA_WEB_PROXY_ENDPOINT: &str = "0.0.0.0:8532";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let matches = command!()
        .arg(
            Arg::new("hooyad-endpoint")
                .long("endpoint")
                .env("HOOYAD_ENDPOINT")
                .default_value(config::DEFAULT_HOOYAD_ENDPOINT),
        )
        .arg(
            Arg::new("proxy-endpoint")
                .long("proxy-endpoint")
                .env("HOOYA_WEB_PROXY_ENDPOINT")
                .default_value(DEFAULT_HOOYA_WEB_PROXY_ENDPOINT),
        )
        .get_matches();

    let state = AState {
        client: ControlClient::connect(format!(
            "http://{}",
            matches.get_one::<String>("hooyad-endpoint").unwrap()
        ))
        .await?,
    };

    let app = Router::new()
        .route("/cid-content/:cid", get(cid_content))
        .route("/cid-thumbnail/:cid/medium", get(cid_thumbnail_medium))
        .route("/cid-thumbnail/:cid/small", get(cid_thumbnail_small))
        .route("/cid-thumbnail/:cid/:long_edge", get(cid_thumbnail))
        .route("/cid-tags/:cid", get(cid_tags))
        .route("/cid-info/:cid", get(cid_info))
        .route("/local-file-page/:page_token", get(local_file_page))
        .with_state(state);

    axum::Server::bind(
        &matches
            .get_one::<String>("proxy-endpoint")
            .unwrap()
            .parse()
            .unwrap(),
    )
    .serve(app.into_make_service())
    .await
    .unwrap();

    Ok(())
}

async fn cid_content(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let (_, cid) = match hooya::cid::decode(&encoded_cid) {
        Ok(cid) => cid,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid CID")
                .into_response()
        }
    };

    let mut client = state.client;
    let mut chunk_stream = client
        .content_at_cid(ContentAtCidRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner();

    let mut body = vec![];
    while let Some(mut m) = chunk_stream.message().await.unwrap() {
        // TODO Stream body
        body.append(&mut m.data);
    }

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
    let mut headers = HeaderMap::new();
    let (_, cid) = match hooya::cid::decode(&encoded_cid) {
        Ok(cid) => cid,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid CID")
                .into_response()
        }
    };

    let mut client = state.client;

    let file_info = client
        .cid_info(CidInfoRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let ext_file = match file_info.ext_file {
        Some(ext_file) => ext_file,
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "No such CID indexed")
                .into_response()
        }
    };

    let thumbs = match ext_file {
        hooya::proto::file::ExtFile::Image(i) => i.thumbnails,
    };

    let thumbnail = closest_thumbnail(&thumbs, 1280);
    let long_edge = if thumbnail.width > thumbnail.height {
        thumbnail.width
    } else {
        thumbnail.height
    }
    .try_into()
    .unwrap();

    let mut chunk_stream = client
        .cid_thumbnail(CidThumbnailRequest {
            source_cid: cid,
            long_edge,
        })
        .await
        .unwrap()
        .into_inner();

    let mut body = vec![];
    while let Some(mut m) = chunk_stream.message().await.unwrap() {
        // TODO Stream body
        body.append(&mut m.data);
    }

    headers.append(
        axum::http::header::CACHE_CONTROL,
        "max-age=31536000, immutable".parse().unwrap(),
    );
    headers.append(axum::http::header::CONTENT_LENGTH, thumbnail.size.into());

    headers.append(
        axum::http::header::CONTENT_TYPE,
        thumbnail.mimetype.parse().unwrap(),
    );

    let save_extension = mimetype_extension(&thumbnail.mimetype).unwrap();
    headers.append(
        axum::http::header::CONTENT_DISPOSITION,
        format!(
            "inline; filename=\"{}_thumb{}.{}\"",
            encoded_cid, long_edge, save_extension
        )
        .parse()
        .unwrap(),
    );

    (headers, body).into_response()
}

async fn cid_thumbnail_small(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    let (_, cid) = match hooya::cid::decode(&encoded_cid) {
        Ok(cid) => cid,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid CID")
                .into_response()
        }
    };

    let mut client = state.client;

    let file_info = client
        .cid_info(CidInfoRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let ext_file = match file_info.ext_file {
        Some(ext_file) => ext_file,
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "No such CID indexed")
                .into_response()
        }
    };

    let thumbs = match ext_file {
        hooya::proto::file::ExtFile::Image(i) => i.thumbnails,
    };

    let thumbnail = closest_thumbnail(&thumbs, 640);
    let long_edge = if thumbnail.width > thumbnail.height {
        thumbnail.width
    } else {
        thumbnail.height
    }
    .try_into()
    .unwrap();

    let mut chunk_stream = client
        .cid_thumbnail(CidThumbnailRequest {
            source_cid: cid,
            long_edge,
        })
        .await
        .unwrap()
        .into_inner();

    let mut body = vec![];
    while let Some(mut m) = chunk_stream.message().await.unwrap() {
        // TODO Stream body
        body.append(&mut m.data);
    }

    headers.append(
        axum::http::header::CACHE_CONTROL,
        "max-age=31536000, immutable".parse().unwrap(),
    );
    headers.append(axum::http::header::CONTENT_LENGTH, thumbnail.size.into());

    headers.append(
        axum::http::header::CONTENT_TYPE,
        thumbnail.mimetype.parse().unwrap(),
    );

    let save_extension = mimetype_extension(&thumbnail.mimetype).unwrap();
    headers.append(
        axum::http::header::CONTENT_DISPOSITION,
        format!(
            "inline; filename=\"{}_thumb{}.{}\"",
            encoded_cid, long_edge, save_extension
        )
        .parse()
        .unwrap(),
    );

    (headers, body).into_response()
}

async fn cid_thumbnail(
    State(state): State<AState>,
    Path((encoded_cid, long_edge)): Path<(String, u32)>,
) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    let (_, cid) = match hooya::cid::decode(&encoded_cid) {
        Ok(cid) => cid,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid CID")
                .into_response()
        }
    };

    let mut client = state.client;

    let file_info = client
        .cid_info(CidInfoRequest { cid: cid.clone() })
        .await
        .unwrap()
        .into_inner()
        .file
        .unwrap();

    let ext_file = match file_info.ext_file {
        Some(ext_file) => ext_file,
        None => {
            return (axum::http::StatusCode::NOT_FOUND, "No such CID indexed")
                .into_response()
        }
    };

    let thumbs = match ext_file {
        hooya::proto::file::ExtFile::Image(i) => i.thumbnails,
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

    let mut chunk_stream = client
        .cid_thumbnail(CidThumbnailRequest {
            source_cid: cid,
            long_edge,
        })
        .await
        .unwrap()
        .into_inner();

    let mut body = vec![];
    while let Some(mut m) = chunk_stream.message().await.unwrap() {
        // TODO Stream body
        body.append(&mut m.data);
    }

    headers.append(
        axum::http::header::CACHE_CONTROL,
        "max-age=31536000, immutable".parse().unwrap(),
    );
    headers.append(axum::http::header::CONTENT_LENGTH, thumb.size.into());

    headers.append(
        axum::http::header::CONTENT_TYPE,
        thumb.mimetype.parse().unwrap(),
    );

    let save_extension = mimetype_extension(&thumb.mimetype).unwrap();
    headers.append(
        axum::http::header::CONTENT_DISPOSITION,
        format!(
            "inline; filename=\"{}_thumb{}.{}\"",
            encoded_cid, long_edge, save_extension
        )
        .parse()
        .unwrap(),
    );

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

async fn cid_tags(
    State(state): State<AState>,
    Path(encoded_cid): Path<String>,
) -> impl IntoResponse {
    let (_, cid) = match hooya::cid::decode(&encoded_cid) {
        Ok(cid) => cid,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid CID")
                .into_response()
        }
    };

    let mut client = state.client;

    let tags: Vec<Tag> = client
        .tags(TagsRequest{
            cid,
        })
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
    let (_, cid) = match hooya::cid::decode(&encoded_cid) {
        Ok(cid) => cid,
        _ => {
            return (axum::http::StatusCode::BAD_REQUEST, "Invalid CID")
                .into_response()
        }
    };

    let mut client = state.client;

    let info: Option<hooya::proto::File> = client
        .cid_info(CidInfoRequest{
            cid,
        })
        .await
        .unwrap()
        .into_inner()
        .file;

    let info = match info {
        Some(i) => i,
        _ => return (axum::http::StatusCode::BAD_REQUEST, "No Info")
                .into_response()
    };

    // Doing this only to translate CID
    let cid = hooya::cid::encode(info.cid);
    let size = info.size;
    let mimetype = info.mimetype;
    let ext_file = info.ext_file;

    let body = CidInfoResponse {
        cid,
        size,
        mimetype,
        ext_file,
    };

    axum::Json(body).into_response()
}

async fn local_file_page(
    State(state): State<AState>,
    Path(page_token): Path<String>,
) -> impl IntoResponse {
    let mut client = state.client;

    let local_file_page_resp = client
        .local_file_page(LocalFilePageRequest{
            oldest_first: false,
            page_token,
            page_size: 100,
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

    let body = LocalFilePageResponse {
        cid,
        next_page_token,
    };

    axum::Json(body).into_response()
}

#[derive(Serialize, Deserialize)]
struct LocalFilePageResponse {
    cid: Vec<String>,
    next_page_token: String,
}

#[derive(Serialize, Deserialize)]
struct CidInfoResponse {
    cid: String,
    size: i64,
    mimetype: Option<String>,
    ext_file: Option<hooya::proto::file::ExtFile>,
}
