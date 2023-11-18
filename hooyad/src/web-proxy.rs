use anyhow::Result;
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
    control_client::ControlClient, CidInfoRequest, ContentAtCidRequest,
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
    let (_, cid) = hooya::cid::decode(&encoded_cid).unwrap();

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
        headers
            .append(axum::http::header::CONTENT_TYPE, mtype.parse().unwrap());
    }

    (headers, body).into_response()
}
