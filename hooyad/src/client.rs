use anyhow::Result;
use clap::{command, value_parser, Arg, ArgAction, Command};
use dotenv::dotenv;
use hooya::proto::{control_client::ControlClient, FileChunk, TagCidRequest};
use std::{fs::File, path::Path};
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
            Command::new("add").arg(Arg::new("file").action(ArgAction::Append)),
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
                .get_many::<String>("file")
                .unwrap_or_default()
                .map(|v| v.as_str())
                .collect::<Vec<_>>();
            for f in &files {
                let fh = File::open(f)?;
                let chunks = hooya::ChunkedReader::new(fh)
                    .map(|c| FileChunk { data: c.unwrap() });
                let resp = client
                    .stream_to_filestore(futures_util::stream::iter(chunks))
                    .await?;
                let cid = resp.into_inner().cid;
                println!(
                    "added {} {}",
                    hooya::cid::encode(cid),
                    Path::new(f).file_name().unwrap().to_str().unwrap()
                );
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
