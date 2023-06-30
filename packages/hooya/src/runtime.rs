use crate::local::{self, FileRow};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

pub struct Runtime {
    pub filestore_path: PathBuf,
    pub db: local::Db,
}

impl Runtime {
    pub async fn new_from_filestore(&self, cid: Vec<u8>) -> Result<()> {
        let cid_store_path = self.derive_store_path(&cid);

        let size: i64 =
            fs::metadata(cid_store_path.clone())?.len().try_into()?; // TODO

        let mimetype = tree_magic::from_filepath(&cid_store_path);

        let f = FileRow {
            cid,
            size,
            mimetype,
        };

        self.db.new_file(f).await?;

        Ok(())
    }

    pub fn derive_store_path(&self, cid: &[u8]) -> PathBuf {
        // TODO May be more useful to keep the encoded version around instead
        // of (de|en)coding it often?
        let encoded_cid = crate::cid::encode(cid);

        // Keep /store kinda uncluttered by dividing data up into dirs
        let final_dir =
            self.filestore_path.join("store").join(&encoded_cid[..6]);

        // eg bafkreifh22[...] is stored at bafkre/bafkreifh22[...]
        final_dir.join(encoded_cid)
    }
}
