use anyhow::Result;
use clap::{command, Arg, ArgAction, Command};
use dotenv::dotenv;
use hooya::proto::{
    control_client::ControlClient, FileChunk, Tag, TagCidRequest,
};
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
            Command::new("tag")
                .arg(Arg::new("cid").required(true))
                .arg(Arg::new("tags").action(ArgAction::Append).required(true)),
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
            let tag_result: Result<Vec<_>> = sub_matches
                .get_many::<String>("tags")
                .unwrap_or_default()
                // TODO Better to do w Clap value_parser
                .map(|v| tag_from_cli(v))
                .collect();
            client
                .tag_cid(TagCidRequest {
                    cid,
                    tags: tag_result?,
                })
                .await?;
        }
        _ => unreachable!("Exhausted list of subcommands"),
    }

    Ok(())
}

fn tag_from_cli(s: &str) -> Result<Tag> {
    let parts = s.split_once(':').ok_or(anyhow::anyhow!(
        "Tag must be qualified with namespaces (namespace:descriptor)"
    ))?;
    Ok(Tag {
        namespace: parts.0.to_string(),
        descriptor: parts.1.to_string(),
    })
}
