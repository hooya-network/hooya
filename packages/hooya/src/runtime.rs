use crate::local::{self, FileRow, ImageRow, TagMapRow, ThumbnailRow};
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

        let inferred = infer::get_from_path(&cid_store_path)?;
        let mimetype = inferred.map(|i| i.to_string());

        let f = FileRow {
            cid: cid.clone(),
            size,
            mimetype: mimetype.clone(),
        };

        self.db.new_file(f).await?;
        self.import_image(cid, &mimetype.unwrap()).await?;

        Ok(())
    }

    pub async fn indexed_file(&self, cid: Vec<u8>) -> Result<File> {
        let file_row = self.db.file_row(cid).await?;

        // The reason for this cute misdirection is that indexed (ie local)
        // File may not always map 1-to-1 with the concept of Files on the network
        let file = File {
            cid: file_row.cid,
            mimetype: file_row.mimetype,
            size: file_row.size,
        };

        Ok(file)
    }

    pub async fn tags(&self, cid: Vec<u8>) -> Result<Vec<Tag>> {
        let tags = self
            .db
            .file_tags(cid)
            .await?
            .iter()
            .map(|r| Tag {
                namespace: r.namespace.clone(),
                descriptor: r.descriptor.clone(),
            })
            .collect();

        Ok(tags)
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

        let prefix = if encoded_cid.len() >= 11 {
            &encoded_cid[..11]
        } else {
            &encoded_cid
        };

        // Keep /store kinda uncluttered by dividing data up into dirs
        let final_dir = self.filestore_path.join("store").join(prefix);

        // eg bafkreifh22[...]fpydri is stored at ydri/bafkreifh22[...]
        Ok(final_dir.join(encoded_cid))
    }

    pub fn derive_thumb_path(&self, cid: &[u8], size: u32) -> Result<PathBuf> {
        // TODO May be more useful to keep the encoded version around instead
        // of (de|en)coding it often?
        let encoded_cid = crate::cid::encode(cid);

        if encoded_cid.is_empty() {
            return Err(anyhow::anyhow!("Unable to derive path for empty CID"));
        }

        let prefix = if encoded_cid.len() >= 11 {
            &encoded_cid[..11]
        } else {
            &encoded_cid
        };

        let final_dir = self
            .filestore_path
            .join("thumbs")
            .join(size.to_string())
            .join(prefix);

        Ok(final_dir.join([encoded_cid, size.to_string()].join("_thumb")))
    }

    pub async fn random_local_file(
        &self,
        count: u32,
    ) -> Result<Vec<crate::proto::File>> {
        let files = self
            .db
            .random_file(count)
            .await?
            .into_iter()
            .map(|f| crate::proto::File {
                cid: f.cid,
                mimetype: f.mimetype,
                size: f.size,
            })
            .collect();

        Ok(files)
    }

    pub async fn local_file_page(
        &self,
        page_size: u32,
        page_token: String,
        oldest_first: bool,
    ) -> Result<(Vec<crate::proto::File>, String)> {
        let offset: u32 = page_token.parse()?;
        let files = self
            .db
            .file_page(page_size, offset, oldest_first)
            .await?
            .into_iter()
            .map(|f| crate::proto::File {
                cid: f.cid,
                mimetype: f.mimetype,
                size: f.size,
            })
            .collect();

        Ok((files, (offset + page_size).to_string()))
    }

    pub async fn import_image(
        &self,
        cid: Vec<u8>,
        mimetype: &str,
    ) -> Result<()> {
        let cid_store_path = self.derive_store_path(&cid)?;
        let decoded_image = crate::image::read(&cid_store_path, mimetype)?;
        let img_width = decoded_image.width();
        let img_height = decoded_image.height();

        self.db
            .new_image(ImageRow {
                cid: cid.clone(),
                height: img_height,
                width: img_width,
                ratio: f64::from(img_width) / f64::from(img_height),
                primary_color: vec![],
                colors: vec![],
            })
            .await?;

        // Thumbnail sizes to generate
        let t_sizes_long_edge = vec![320, 640, 960, 1280, 1920];

        // Read file and thumbnail for every size listed
        for t_size_long_edge in t_sizes_long_edge {
            if img_width < t_size_long_edge && img_height < t_size_long_edge {
                continue;
            }

            let thumb_store_path =
                self.derive_thumb_path(&cid, t_size_long_edge)?;

            let parent = thumb_store_path.parent().unwrap();
            if !parent.is_dir() {
                std::fs::create_dir_all(parent)?;
            }

            let (thumb_height, thumb_width) = crate::image::thumbnail(
                &cid_store_path,
                &thumb_store_path,
                mimetype,
                t_size_long_edge,
            )?;

            let fh = std::fs::File::open(thumb_store_path)?;
            let size = fh.metadata()?.len().try_into().unwrap(); // TODO

            let chunks = crate::ChunkedReader::new(fh);
            let mut sha_context = crate::cid::new_digest_context();

            for c in chunks {
                sha_context.update(&c?);
            }

            let thumb_cid = crate::cid::wrap_digest(sha_context.finish())?;

            self.db
                .new_thumbnail(ThumbnailRow {
                    cid: thumb_cid,
                    size,
                    mimetype: mimetype.to_string(),
                    source_cid: cid.clone(),
                    ratio: f64::from(img_width) / f64::from(img_height),
                    height: thumb_height.into(),
                    width: thumb_width.into(),
                    is_animated: false,
                })
                .await?;
        }

        Ok(())
    }
}
