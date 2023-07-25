use anyhow::Result;
use futures_util::future::BoxFuture;
use std::{fs::File, path::Path};

use tonic::transport::Channel;

use crate::proto::{control_client::ControlClient, FileChunk};

pub async fn stream_file_to_remote_filestore(
    mut client: ControlClient<Channel>,
    local_file: &Path,
) -> Result<()> {
    let fh = File::open(local_file)?;

    if fh.metadata()?.len() == 0 {
        println!("Not streaming empty file {}", local_file.file_name().unwrap().to_str().unwrap());
        return Ok(())
    }

    let chunks =
        crate::ChunkedReader::new(fh).map(|c| FileChunk { data: c.unwrap() });
    let resp = client
        .stream_to_filestore(futures_util::stream::iter(chunks))
        .await
        .map_err(anyhow::Error::new)?;
    let cid = resp.into_inner().cid;
    println!(
        "added {} {}",
        crate::cid::encode(cid),
        Path::new(local_file).file_name().unwrap().to_str().unwrap()
    );

    Ok(())
}

pub fn stream_dir_to_remote_filestore(
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
