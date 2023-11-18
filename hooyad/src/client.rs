use anyhow::Result;
use clap::{command, value_parser, Arg, ArgAction, Command};
use dotenv::dotenv;
use hooya::proto::{
    control_client::ControlClient, ContentAtCidRequest, TagCidRequest,
};
use std::path::{Path, PathBuf};
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
            Command::new("add")
                .arg(
                    Arg::new("just-hash")
                        .action(ArgAction::SetTrue)
                        .long("just-hash"),
                )
                .arg(
                    Arg::new("unlink")
                        .action(ArgAction::SetTrue)
                        .long("unlink"),
                )
                .arg(
                    Arg::new("init-tag")
                        .action(ArgAction::Append)
                        .value_parser(value_parser!(hooya::proto::Tag))
                        .long("init-tag"),
                )
                .arg(
                    Arg::new("files")
                        .action(ArgAction::Append)
                        .value_parser(value_parser!(PathBuf)),
                ),
        )
        .subcommand(
            Command::new("add-dir")
                .arg(
                    Arg::new("unlink")
                        .action(ArgAction::SetTrue)
                        .long("unlink"),
                )
                .arg(
                    Arg::new("init-tag")
                        .action(ArgAction::Append)
                        .value_parser(value_parser!(hooya::proto::Tag))
                        .long("init-tag"),
                )
                .arg(
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
        .subcommand(Command::new("dl").arg(Arg::new("cid").required(true)))
        .get_matches();

    let mut client = ControlClient::connect(format!(
        "http://{}",
        matches.get_one::<String>("endpoint").unwrap()
    ))
    .await?;

    match matches.subcommand() {
        Some(("add", sub_matches)) => {
            use std::fs::File;
            let just_hash =
                *sub_matches.get_one::<bool>("just-hash").unwrap_or(&false);
            let unlink =
                *sub_matches.get_one::<bool>("unlink").unwrap_or(&false);
            let init_tags: Vec<hooya::proto::Tag> = sub_matches
                .get_many::<hooya::proto::Tag>("init-tag")
                .unwrap_or_default()
                .cloned()
                .collect();
            let files = sub_matches
                .get_many::<PathBuf>("files")
                .unwrap_or_default()
                .collect::<Vec<_>>();

            for f in &files {
                if just_hash {
                    let fh = File::open(f)?;
                    let chunks = hooya::ChunkedReader::new(fh);
                    let mut sha_context = hooya::cid::new_digest_context();

                    for c in chunks {
                        sha_context.update(&c?);
                    }
                    let digest = hooya::cid::wrap_digest(sha_context.finish())?;
                    println!(
                        "hashed {} {}",
                        hooya::cid::encode(digest),
                        Path::new(f).file_name().unwrap().to_str().unwrap()
                    );
                    continue;
                }
                hooya::client::stream_file_to_remote_filestore(
                    client.clone(),
                    f,
                    unlink,
                    init_tags.clone(),
                )
                .await?;
            }
        }
        Some(("add-dir", sub_matches)) => {
            let dirs = sub_matches
                .get_many::<PathBuf>("dirs")
                .unwrap_or_default()
                .collect::<Vec<_>>();
            let unlink =
                *sub_matches.get_one::<bool>("unlink").unwrap_or(&false);

            let init_tags: Vec<hooya::proto::Tag> = sub_matches
                .get_many::<hooya::proto::Tag>("init-tag")
                .unwrap_or_default()
                .cloned()
                .collect();

            for d in &dirs {
                hooya::client::stream_dir_to_remote_filestore(
                    client.clone(),
                    d,
                    unlink,
                    init_tags.clone(),
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
            let mut chunk_stream = client
                .content_at_cid(ContentAtCidRequest { cid })
                .await?
                .into_inner();

            // For now just write to a file in the current path
            let mut file = File::create(encoded_cid)?;

            while let Some(m) = chunk_stream.message().await? {
                file.write_all(&m.data)?;
            }
        }
        _ => unreachable!("Exhausted list of subcommands"),
    }

    Ok(())
}
