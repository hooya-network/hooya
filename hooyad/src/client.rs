use hooya::proto::{control_client::ControlClient, VersionRequest};
mod config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::Config::from_env_and_args();

    let mut client =
        ControlClient::connect(format!("http://{}", cfg.endpoint)).await?;
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
    Ok(())
}
