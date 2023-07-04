use crate::local::{self, FileRow, TagMapRow};
use crate::proto::{File, Tag};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

pub struct Runtime {
    pub filestore_path: PathBuf,
    pub db: local::Db,
}

impl Runtime {
    pub async fn new_from_filestore(&self, cid: Vec<u8>) -> Result<()> {
        let cid_store_path = self.derive_store_path(&cid)?;

        let size: i64 =
            fs::metadata(cid_store_path.clone())?.len().try_into()?; // TODO

        let mimetype = infer::get_from_path(&cid_store_path)?
            .ok_or(anyhow::anyhow!("Failed to infer file mimetype"))?
            .to_string();

        let f = FileRow {
            cid,
            size,
            mimetype,
        };

        self.db.new_file(f).await?;

        Ok(())
    }

    pub async fn indexed_file(&self, cid: Vec<u8>) -> Result<File> {
        let file_row = self.db.file_row(cid).await?;

        // The reason for this cute misdirection is that indexed (ie local)
        // File may not always map 1-to-1 with the concept of Files on the network
        let file = File {
            cid: file_row.cid,
            mimetype: file_row.mimetype,
            size: file_row.size.try_into()?,
        };

        Ok(file)
    }

    pub async fn tag_cid(&self, cid: Vec<u8>, tags: Vec<Tag>) -> Result<()> {
        let tags_len = tags.len();
        let mut tag_maps =
            self.make_tag_map_rows(cid.clone(), tags.clone()).await?;

        // Cheaper than doing it 1-by-1
        if tag_maps.len() != tags_len {
            self.db.new_tag_vocab(tags.clone()).await?;
            tag_maps = self.make_tag_map_rows(cid, tags).await?;
        }

        self.db.new_tag_map(&tag_maps).await?;
        Ok(())
    }

    async fn make_tag_map_rows(
        &self,
        cid: Vec<u8>,
        tags: Vec<Tag>,
    ) -> Result<Vec<TagMapRow>> {
        let tag_ids = self.db.lookup_tag_id(tags.clone()).await?;

        let rows = tag_ids
            .iter()
            .map(|t| TagMapRow {
                file_cid: cid.clone(),
                tag_id: t.id,
                added: None,
                reason: 0, // TODO enum w "added by node opeartor" reason as 0
            })
            .collect::<Vec<TagMapRow>>();
        Ok(rows)
    }

    pub fn derive_store_path(&self, cid: &[u8]) -> Result<PathBuf> {
        // TODO May be more useful to keep the encoded version around instead
        // of (de|en)coding it often?
        let encoded_cid = crate::cid::encode(cid);

        if encoded_cid.is_empty() {
            return Err(anyhow::anyhow!("Unable to derive path for empty CID"));
        }

        let prefix = if encoded_cid.len() >= 6 {
            &encoded_cid[encoded_cid.len() - 6..]
        } else {
            &encoded_cid
        };

        // Keep /store kinda uncluttered by dividing data up into dirs
        let final_dir = self.filestore_path.join("store").join(prefix);

        // eg bafkreifh22[...]fpydri is stored at ydri/bafkreifh22[...]
        Ok(final_dir.join(encoded_cid))
    }
}
