use anyhow::Result;
use clap::{command, value_parser, Arg, ArgAction, Command};
use dotenv::dotenv;
use futures_util::future::BoxFuture;
use hooya::proto::{control_client::ControlClient, FileChunk, TagCidRequest};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use tonic::transport::Channel;
mod config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    let matches = command!()
        .subcommand_required(true)
        .arg(
            Arg::new("endpoint")
                .long("endpoint")
                .env("HOOYAD_ENDPOINT")
                .default_value(config::DEFAULT_HOOYAD_ENDPOINT),
        )
        .subcommand(
            Command::new("add").arg(
                Arg::new("files")
                    .action(ArgAction::Append)
                    .value_parser(value_parser!(PathBuf)),
            ),
        )
        .subcommand(
            Command::new("add-dir").arg(
                Arg::new("dirs")
                    .action(ArgAction::Append)
                    .value_parser(value_parser!(PathBuf)),
            ),
        )
        .subcommand(
            Command::new("tag").arg(Arg::new("cid").required(true)).arg(
                Arg::new("tags")
                    .action(ArgAction::Append)
                    .required(true)
                    .value_parser(value_parser!(hooya::proto::Tag)),
            ),
        )
        .get_matches();

    let mut client = ControlClient::connect(format!(
        "http://{}",
        matches.get_one::<String>("endpoint").unwrap()
    ))
    .await?;

    match matches.subcommand() {
        Some(("add", sub_matches)) => {
            let files = sub_matches
                .get_many::<PathBuf>("files")
                .unwrap_or_default()
                .collect::<Vec<_>>();
            for f in &files {
                stream_file_to_remote_filestore(client.clone(), f).await?;
            }
        }
        Some(("add-dir", sub_matches)) => {
            let dirs = sub_matches
                .get_many::<PathBuf>("dirs")
                .unwrap_or_default()
                .collect::<Vec<_>>();
            for d in &dirs {
                stream_dir_to_remote_filestore(client.clone(), d).await?;
            }
        }
        Some(("tag", sub_matches)) => {
            let encoded_cid = sub_matches.get_one::<String>("cid").unwrap();
            let (_, cid) = hooya::cid::decode(encoded_cid)?;
            let tags = sub_matches
                .get_many::<hooya::proto::Tag>("tags")
                .unwrap_or_default()
                .cloned()
                .collect();
            client.tag_cid(TagCidRequest { cid, tags }).await?;
        }
        _ => unreachable!("Exhausted list of subcommands"),
    }

    Ok(())
}

async fn stream_file_to_remote_filestore(
    mut client: ControlClient<Channel>,
    local_file: &Path,
) -> Result<()> {
    let fh = File::open(local_file)?;
    let chunks =
        hooya::ChunkedReader::new(fh).map(|c| FileChunk { data: c.unwrap() });
    let resp = client
        .stream_to_filestore(futures_util::stream::iter(chunks))
        .await
        .map_err(anyhow::Error::new)?;
    let cid = resp.into_inner().cid;
    println!(
        "added {} {}",
        hooya::cid::encode(cid),
        Path::new(local_file).file_name().unwrap().to_str().unwrap()
    );

    Ok(())
}

fn stream_dir_to_remote_filestore(
    client: ControlClient<Channel>,
    local_dir: &Path,
) -> BoxFuture<Result<()>> {
    Box::pin(async move {
        for entry in std::fs::read_dir(local_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stream_dir_to_remote_filestore(client.clone(), &path).await?;
            } else {
                stream_file_to_remote_filestore(client.clone(), &path).await?;
            }
        }
        Ok(())
    })
}
