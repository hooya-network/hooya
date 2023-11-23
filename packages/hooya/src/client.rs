use anyhow::Result;
use futures_util::future::BoxFuture;
use std::{fs::File, path::Path};

use tonic::transport::Channel;

use crate::proto::{control_client::ControlClient, FileChunk};

pub async fn stream_file_to_remote_filestore(
    mut client: ControlClient<Channel>,
    local_file: &Path,
    unlink: bool,
    cont_inue: bool,
    init_tags: Vec<crate::proto::Tag>,
) -> Result<()> {
    {
        let fh = File::open(local_file)?;

        if fh.metadata()?.len() == 0 {
            println!(
                "Not streaming empty file {}",
                local_file.file_name().unwrap().to_str().unwrap()
            );
            return Ok(());
        }

        let chunks = crate::ChunkedReader::new(fh)
            .map(|c| FileChunk { data: c.unwrap() });
        let resp = client
            .stream_to_filestore(futures_util::stream::iter(chunks))
            .await
            .map_err(|e| {
                anyhow::format_err!(
                    "Error adding {}: {}",
                    local_file.to_string_lossy(),
                    e
                )
            });

        let resp = match resp {
            Ok(r) => r,
            Err(e) => if cont_inue {
                eprintln!("{}", e);
                return Ok(())
            } else {
                return Err(e)
            }
        };

        let cid = resp.into_inner().cid;
        client
            .tag_cid(crate::proto::TagCidRequest {
                cid: cid.clone(),
                tags: init_tags,
            })
            .await?;

        println!(
            "added {} {}",
            crate::cid::encode(cid),
            Path::new(local_file).file_name().unwrap().to_str().unwrap()
        );
    }

    if unlink {
        std::fs::remove_file(local_file)?;
    }

    Ok(())
}

pub fn stream_dir_to_remote_filestore(
    client: ControlClient<Channel>,
    local_dir: &Path,
    unlink: bool,
    cont_inue: bool,
    init_tags: Vec<crate::proto::Tag>,
) -> BoxFuture<Result<()>> {
    Box::pin(async move {
        for entry in std::fs::read_dir(local_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stream_dir_to_remote_filestore(
                    client.clone(),
                    &path,
                    unlink,
                    cont_inue,
                    init_tags.clone(),
                )
                .await?;
            } else {
                stream_file_to_remote_filestore(
                    client.clone(),
                    &path,
                    unlink,
                    cont_inue,
                    init_tags.clone(),
                )
                .await?;
            }
        }
        Ok(())
    })
}
