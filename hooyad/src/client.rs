use anyhow::Result;
use clap::{command, value_parser, Arg, ArgAction, Command};
use dotenv::dotenv;
use hooya::proto::{control_client::ControlClient, TagCidRequest, ContentAtCidRequest};
use std::path::PathBuf;
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
        .subcommand(
            Command::new("dl").arg(Arg::new("cid").required(true))
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
                hooya::client::stream_file_to_remote_filestore(
                    client.clone(),
                    f,
                )
                .await?;
            }
        }
        Some(("add-dir", sub_matches)) => {
            let dirs = sub_matches
                .get_many::<PathBuf>("dirs")
                .unwrap_or_default()
                .collect::<Vec<_>>();
            for d in &dirs {
                hooya::client::stream_dir_to_remote_filestore(
                    client.clone(),
                    d,
                )
                .await?;
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
        Some(("dl", sub_matches)) => {
            use std::fs::File;
            use std::io::Write;

            let encoded_cid = sub_matches.get_one::<String>("cid").unwrap();
            let (_, cid) = hooya::cid::decode(encoded_cid)?;
            let mut chunk_stream = client.content_at_cid(ContentAtCidRequest { cid }).await?.into_inner();

            // For now just write to a file in the current path
            let mut file = File::create(encoded_cid)?;

            while let Some(m) = chunk_stream.message().await? {
                file.write(&m.data)?;
            }
        }
        _ => unreachable!("Exhausted list of subcommands"),
    }

    Ok(())
}
