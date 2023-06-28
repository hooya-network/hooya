use clap::{command, Arg, ArgAction, Command};
use dotenv::dotenv;
use hooya::proto::{control_client::ControlClient, VersionRequest};
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
    let request = tonic::Request::new(VersionRequest {});
    let response = client.version(request).await?.into_inner();

    let ver = semver::Version {
        major: response.major_version,
        minor: response.minor_version,
        patch: response.patch_version,
        pre: semver::Prerelease::new(&response.pre_version).unwrap(),
        build: semver::BuildMetadata::EMPTY,
    };
    println!("Connected to remote hooyad instance {}", ver);

    match matches.subcommand() {
        Some(("add", sub_matches)) => {
            let files = sub_matches
                .get_many::<String>("file")
                .unwrap_or_default()
                .map(|v| v.as_str())
                .collect::<Vec<_>>();
            println!("{:?}", files);
        }
        _ => unreachable!("Exhausted list of subcommands"),
    }

    Ok(())
}
