use clap::{command, Arg, ArgAction, Command};
use dotenv::dotenv;
use hooya::proto::{control_client::ControlClient, FileChunk};
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
        _ => unreachable!("Exhausted list of subcommands"),
    }

    Ok(())
}
