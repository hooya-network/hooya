use hooya::proto::{control_client::ControlClient, VersionRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ControlClient::connect("http://[::1]:50051").await?;
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
