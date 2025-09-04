use crate::chatroom::Chatroom;
use crate::keys;
use crate::local::{
    self, FileRow, ImageRow, TagMapRow, ThumbnailRow, VideoRow,
};
use crate::proto::{File, ProcessingStatus, Tag, Thumbnail};
use anyhow::Result;
use hooya_config::HooyaConfig;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sylow::KeyPair;
use tokio::sync::{broadcast, mpsc, RwLock, Semaphore};

pub fn sanitize_filestore_path(
    filestore_path: &Path,
    subdirectory: &str,
    filename: &str,
) -> Result<PathBuf> {
    let target_dir = filestore_path.join(subdirectory);
    let target_file = target_dir.join(filename);

    if let Some(parent) = target_file.parent() {
        if parent != target_dir {
            return Err(anyhow::anyhow!(
                "invalid path: directory traversal attempt"
            ));
        }
    } else {
        return Err(anyhow::anyhow!("invalid path: no parent directory"));
    }

    Ok(target_file)
}

pub struct Runtime<T: local::DatabaseBackend> {
    pub filestore_path: PathBuf,
    pub db: T,
    pub instance_events: broadcast::Sender<InstanceEvent>,
    pub processing_cids: Arc<RwLock<HashSet<Vec<u8>>>>,
    pub node_keypair: KeyPair,
    pub consensus_keypair: Option<KeyPair>,
    pub config: HooyaConfig,
    pub cpu_semaphore: Semaphore,
    pub mesh_tx: mpsc::Sender<crate::mesh_network::OutgoingMessage>,
    pub chatroom: Arc<Chatroom>,
}

#[derive(Clone, Debug)]
pub struct InstanceEvent {
    pub event_type: InstanceEventType,
}

#[derive(Clone, Debug)]
pub enum InstanceEventType {
    Processing(ProcessingEvent),
    Chat(ChatEvent),
}

#[derive(Clone, Debug)]
pub struct ProcessingEvent {
    pub cid: Vec<u8>,
    pub event_type: ProcessingEventType,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ProcessingEventType {
    Started,
    ThumbnailGenerated { long_edge: u32, mimetype: String },
    VideoPreviewGenerated { long_edge: u32, mimetype: String },
    Finished,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ChatEvent {
    pub channel: String,
    pub content: String,
    pub node_id: String,
    pub signature: Vec<u8>,
}

impl<T: local::DatabaseBackend> Runtime<T> {
    pub async fn import_basic_file_record(
        &self,
        cid: Vec<u8>,
    ) -> Result<crate::proto::File> {
        let cid_store_path = self.derive_store_path(&cid)?;

        let size: i64 =
            fs::metadata(cid_store_path.clone())?.len().try_into()?;

        let inferred = infer::get_from_path(&cid_store_path)?;
        let mimetype = inferred.map(|i| i.to_string());

        let f = FileRow {
            cid: cid.clone(),
            size,
            mimetype: mimetype.clone(),
        };

        self.db.new_file(f).await?;

        Ok(crate::proto::File {
            cid: cid.clone(),
            size,
            mimetype,
            processing_status: ProcessingStatus::ProcessingStarted as i32,
            ext_file: None, // not processed yet sooo
            tags: vec![],   // no tags yet
        })
    }

    pub async fn process_file_background(&self, cid: Vec<u8>) -> Result<()> {
        let cid_store_path = self.derive_store_path(&cid)?;
        let inferred = infer::get_from_path(&cid_store_path)?;

        // Extract additional detail about the file given its type
        if let Some(inferred_mimetype) = inferred {
            let mimetype = inferred_mimetype.to_string();
            match inferred_mimetype.matcher_type() {
                infer::MatcherType::Image => {
                    self.import_image(cid, &mimetype).await?
                }
                infer::MatcherType::Video => {
                    self.import_video(cid, &mimetype).await?
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub fn emit_processing_event(
        &self,
        cid: Vec<u8>,
        event_type: ProcessingEventType,
        error: Option<String>,
    ) {
        let event = ProcessingEvent {
            cid: cid.clone(),
            event_type: event_type.clone(),
            error_message: error,
        };

        // update processing cache
        tokio::spawn({
            let processing_cids = self.processing_cids.clone();
            async move {
                match event_type {
                    ProcessingEventType::Started => {
                        let mut cache = processing_cids.write().await;
                        cache.insert(cid);
                    }
                    ProcessingEventType::ThumbnailGenerated { .. }
                    | ProcessingEventType::VideoPreviewGenerated { .. } => {
                        // don't remove from cache yet - still processing other sizes
                    }
                    ProcessingEventType::Finished
                    | ProcessingEventType::Failed => {
                        let mut cache = processing_cids.write().await;
                        cache.remove(&cid);
                    }
                }
            }
        });

        // ignore if no listeners
        let instance_event = InstanceEvent {
            event_type: InstanceEventType::Processing(event),
        };
        let _ = self.instance_events.send(instance_event);
    }

    pub fn emit_chat_event(
        &self,
        channel: String,
        content: String,
        node_id: String,
        signature: Vec<u8>,
    ) {
        let chat_event = ChatEvent {
            channel,
            content,
            node_id,
            signature,
        };

        let instance_event = InstanceEvent {
            event_type: InstanceEventType::Chat(chat_event),
        };

        // ignore if no listeners
        let _ = self.instance_events.send(instance_event);
    }

    pub async fn get_processing_status(&self, cid: &[u8]) -> ProcessingStatus {
        let processing_cids = self.processing_cids.read().await;
        if processing_cids.contains(cid) {
            ProcessingStatus::ProcessingStarted
        } else {
            ProcessingStatus::ProcessingFinished
        }
    }

    pub fn get_processing_status_sync(&self, cid: &[u8]) -> ProcessingStatus {
        // try non-blocking read, default to finished if lock is contended
        match self.processing_cids.try_read() {
            Ok(processing_cids) => {
                if processing_cids.contains(cid) {
                    ProcessingStatus::ProcessingStarted
                } else {
                    ProcessingStatus::ProcessingFinished
                }
            }
            Err(_) => ProcessingStatus::ProcessingFinished, // conservative default
        }
    }

    pub async fn import_from_filestore(&self, cid: Vec<u8>) -> Result<()> {
        self.import_basic_file_record(cid.clone()).await?;
        self.process_file_background(cid).await?;
        Ok(())
    }

    pub async fn indexed_file(&self, cid: Vec<u8>) -> Result<File> {
        let file_row = self.db.file_row(cid.clone()).await?;
        let ext_file = if let Some(mimetype) = file_row.mimetype.clone() {
            self.ext_file_info(cid, &mimetype).await?
        } else {
            None
        };

        // The reason for this cute misdirection is that indexed (ie local)
        // File may not always map 1-to-1 with the concept of Files on the network
        // fetch tags for this file
        let tag_rows = self.db.file_tags(file_row.cid.clone()).await?;
        let tags = tag_rows
            .into_iter()
            .map(|tag_row| Tag {
                namespace: tag_row.namespace,
                descriptor: tag_row.descriptor,
            })
            .collect();

        let file = File {
            cid: file_row.cid.clone(),
            mimetype: file_row.mimetype,
            size: file_row.size,
            processing_status: self.get_processing_status_sync(&file_row.cid)
                as i32,
            ext_file,
            tags,
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

    pub async fn untag_cid(&self, cid: Vec<u8>, tags: Vec<Tag>) -> Result<()> {
        let tag_rows = self.db.lookup_tag_id(tags).await?;
        let tag_ids: Vec<i32> = tag_rows.iter().map(|t| t.id).collect();

        self.db.remove_tag_map(cid, &tag_ids).await?;
        Ok(())
    }

    // batch methods that process multiple CIDs in a single transaction
    pub async fn batch_tag_cids(
        &self,
        cid_tag_pairs: Vec<(Vec<u8>, Vec<Tag>)>,
    ) -> Result<Vec<bool>> {
        if cid_tag_pairs.is_empty() {
            return Ok(vec![]);
        }

        // collect all unique tags and ensure they exist in vocabulary
        let mut seen_tags = std::collections::HashSet::new();
        let mut all_tags = Vec::new();
        for (_, tags) in &cid_tag_pairs {
            for tag in tags {
                let key = (&tag.namespace, &tag.descriptor);
                if seen_tags.insert(key) {
                    all_tags.push(tag.clone());
                }
            }
        }

        if !all_tags.is_empty() {
            self.db.new_tag_vocab(all_tags).await?;
        }

        // build all tag map rows
        let mut all_tag_maps = Vec::new();
        let mut results = Vec::new();

        for (cid, tags) in cid_tag_pairs {
            match self.make_tag_map_rows(cid.clone(), tags).await {
                Ok(tag_maps) => {
                    all_tag_maps.extend(tag_maps);
                    results.push(true);
                }
                Err(_) => {
                    results.push(false);
                }
            }
        }

        // batch insert all tag maps in one transaction
        if !all_tag_maps.is_empty() {
            self.db.batch_new_tag_maps(&all_tag_maps).await?;
        }

        Ok(results)
    }

    pub async fn batch_untag_cids(
        &self,
        cid_tag_pairs: Vec<(Vec<u8>, Vec<Tag>)>,
    ) -> Result<Vec<bool>> {
        if cid_tag_pairs.is_empty() {
            return Ok(vec![]);
        }

        // collect all removals to be done in one transaction
        let mut all_removals = Vec::new();
        let mut results = Vec::new();

        for (cid, tags) in cid_tag_pairs {
            match self.db.lookup_tag_id(tags).await {
                Ok(tag_rows) => {
                    for tag_row in tag_rows {
                        all_removals.push((cid.clone(), tag_row.id));
                    }
                    results.push(true);
                }
                Err(_) => {
                    results.push(false);
                }
            }
        }

        // batch remove all tag maps in one transaction
        if !all_removals.is_empty() {
            self.db.batch_remove_tag_maps(&all_removals).await?;
        }

        Ok(results)
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
                cid: f.cid.clone(),
                mimetype: f.mimetype,
                size: f.size,
                processing_status: self.get_processing_status_sync(&f.cid)
                    as i32,
                ext_file: None, // TODO INNER JOIN
                tags: vec![],   // tags not fetched in this context
            })
            .collect();

        Ok(files)
    }

    pub async fn suggest_tags_within_namespace(
        &self,
        existing_tags: &[crate::proto::TagQuery],
        within_namespace: String,
        incomplete_descriptor: String,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<Vec<crate::proto::TagSuggestion>> {
        // CASE - User was typing a descriptor with a qualified namespace
        let mut suggestions = self
            .db
            .get_descriptors_that_start_with(
                existing_tags,
                Some(within_namespace),
                &incomplete_descriptor,
                visibility_filter,
            )
            .await?;

        suggestions.sort_unstable_by(|a, b| b.count.cmp(&a.count));

        let ret = suggestions
            .into_iter()
            .map(|s| crate::proto::TagSuggestion {
                namespace: Some(s.namespace),
                descriptor: s.descriptor,
                count: s.count,
                distance: 0, // TODO Levenshtein distance
            })
            .collect();

        Ok(ret)
    }

    pub async fn suggest_all_tags(
        &self,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<Vec<crate::proto::TagSuggestion>> {
        // CASE - User was typing a descriptor with a qualified namespace
        let mut suggestions =
            self.db.get_most_popular_tags(visibility_filter).await?;

        suggestions.sort_unstable_by(|a, b| b.count.cmp(&a.count));

        let ret = suggestions
            .into_iter()
            .map(|s| crate::proto::TagSuggestion {
                namespace: Some(s.namespace),
                descriptor: s.descriptor,
                count: s.count,
                distance: 0, // TODO Levenshtein distance
            })
            .collect();

        Ok(ret)
    }
    pub async fn suggest_tags_without_namespace(
        &self,
        existing_tags: &[crate::proto::TagQuery],
        suggest_string: &str,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<Vec<crate::proto::TagSuggestion>> {
        // Hashed on tag ID because we don't want to return the same tag twice
        let mut suggestions: HashMap<i32, crate::local::TagRowCount> =
            HashMap::new();
        let mut insert_suggest_hash = |s: crate::local::TagRowCount| {
            suggestions.insert(s.id, s);
        };

        // CASE - User was typing a namespace
        self.db
            .get_most_popular_tags_within_namespace_that_starts_with(
                existing_tags,
                suggest_string,
                visibility_filter,
            )
            .await?
            .into_iter()
            .for_each(&mut insert_suggest_hash);

        // CASE - User was typing a descriptor without qualifying a namespace
        self.db
            .get_descriptors_that_start_with(
                existing_tags,
                None,
                suggest_string,
                visibility_filter,
            )
            .await?
            .into_iter()
            .for_each(&mut insert_suggest_hash);

        let mut suggest_vec = suggestions
            .values()
            .cloned()
            .collect::<Vec<crate::local::TagRowCount>>();
        suggest_vec.sort_unstable_by(|a, b| b.count.cmp(&a.count));

        // suggestions.shrink_to(len);

        let ret = suggest_vec
            .into_iter()
            .map(|s| crate::proto::TagSuggestion {
                namespace: Some(s.namespace),
                descriptor: s.descriptor,
                count: s.count,
                distance: 0, // TODO Levenshtein distance
            })
            .collect();

        Ok(ret)
    }

    pub async fn search_page(
        &self,
        query: crate::proto::SearchQuery,
        page_size: u32,
        page_token: String,
        sort_order: i32,
        reverse_order: bool,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<(Vec<crate::proto::File>, String, u32)> {
        // I don't see a reason to not work with pages as simply numbers
        let page_number: u32 = page_token.parse()?;

        let (mut files, final_page_token) = self
            .db
            .files_page(
                Some(query),
                page_size,
                page_number,
                sort_order,
                reverse_order,
                visibility_filter,
            )
            .await?;

        // update processing status with live cache
        for file in &mut files {
            file.processing_status =
                self.get_processing_status_sync(&file.cid) as i32;
        }

        let next_page_token = if page_number >= final_page_token {
            "".to_string()
        } else {
            (page_number + 1).to_string()
        };

        Ok((files, next_page_token, final_page_token))
    }

    pub async fn all_files_page(
        &self,
        page_size: u32,
        page_token: String,
        sort_order: i32,
        reverse_order: bool,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<(Vec<crate::proto::File>, String, u32)> {
        // I don't see a reason to not work with pages as simply numbers
        let page_number: u32 = page_token.parse()?;
        let (mut files, final_page_token) = self
            .db
            .files_page(
                None,
                page_size,
                page_number,
                sort_order,
                reverse_order,
                visibility_filter,
            )
            .await?;

        // update processing status with live cache
        for file in &mut files {
            file.processing_status =
                self.get_processing_status_sync(&file.cid) as i32;
        }

        let next_page_token = if page_number >= final_page_token {
            "".to_string()
        } else {
            (page_number + 1).to_string()
        };

        Ok((files, next_page_token, final_page_token))
    }

    pub async fn all_tags_page(
        &self,
        page_size: u32,
        page_token: String,
        sort_order: i32,
        reverse_order: bool,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<(Vec<crate::proto::TagInfo>, String, u32)> {
        // I don't see a reason to not work with pages as simply numbers
        let page_number: u32 = page_token.parse()?;
        let (tags, final_page_token) = self
            .db
            .tags_page(
                page_size,
                page_number,
                sort_order,
                reverse_order,
                visibility_filter,
            )
            .await?;

        let tags_info = tags
            .into_iter()
            .map(|t| crate::proto::TagInfo {
                count: t.count,
                namespace: t.namespace,
                descriptor: t.descriptor,
            })
            .collect();

        let next_page_token = if page_number >= final_page_token {
            "".to_string()
        } else {
            (page_number + 1).to_string()
        };

        Ok((tags_info, next_page_token, final_page_token))
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
                cid: f.cid.clone(),
                mimetype: f.mimetype,
                size: f.size,
                processing_status: self.get_processing_status_sync(&f.cid)
                    as i32,
                ext_file: None, // TODO INNER JOIN
                tags: vec![],   // tags not fetched in this context
            })
            .collect();

        Ok((files, (offset + page_size).to_string()))
    }

    pub async fn import_video(
        &self,
        cid: Vec<u8>,
        mimetype: &str,
    ) -> Result<()> {
        let cid_store_path = self.derive_store_path(&cid)?;
        let cid_hex = hex::encode(&cid);

        tracing::info!(
            "processing video file: cid={}, mimetype={}, path={:?}",
            cid_hex,
            mimetype,
            cid_store_path
        );

        let video_metadata = crate::video::extract_video_metadata(
            &cid_store_path,
        )
        .map_err(|e| {
            tracing::error!(
                "failed to extract video metadata for cid={}: {}",
                cid_hex,
                e
            );

            e
        })?;
        let video_width = video_metadata.width;
        let video_height = video_metadata.height;
        let video_duration = video_metadata.duration;

        self.db
            .new_video(VideoRow {
                cid: cid.clone(),
                height: video_height,
                width: video_width,
                duration: video_duration,
                ratio: f64::from(video_width) / f64::from(video_height),
            })
            .await?;

        // Clear out old thumbnails as this generates new ones
        self.db.delete_old_thumbnails(cid.clone()).await?;

        // Thumbnail sizes to generate
        let t_sizes_long_edge = vec![320, 640, 1280];

        for t_size_long_edge in t_sizes_long_edge {
            if video_width < t_size_long_edge && video_height < t_size_long_edge
            {
                continue;
            }

            let thumb_store_path =
                self.derive_thumb_path(&cid, t_size_long_edge)?;

            let parent = thumb_store_path.parent().unwrap();
            if !parent.is_dir() {
                std::fs::create_dir_all(parent)?;
            }

            let (preview_height, preview_width) = {
                let _permit = self.cpu_semaphore.acquire().await?;
                tokio::task::spawn_blocking({
                    let cid_store_path = cid_store_path.clone();
                    let thumb_store_path = thumb_store_path.clone();
                    move || {
                        crate::video::preview(
                            &cid_store_path,
                            &thumb_store_path,
                            t_size_long_edge,
                        )
                    }
                })
                .await??
            };

            let (thumb_cid, size) = tokio::task::spawn_blocking({
                let thumb_store_path = thumb_store_path.clone();
                move || -> Result<(Vec<u8>, i64)> {
                    let fh = std::fs::File::open(thumb_store_path)?;
                    let size = fh.metadata()?.len().try_into().unwrap(); // TODO

                    let chunks = crate::ChunkedReader::new(fh);
                    let mut sha_context = crate::cid::new_digest_context();

                    for c in chunks {
                        sha_context.update(&c?);
                    }

                    let thumb_cid =
                        crate::cid::wrap_digest(sha_context.finish())?;
                    Ok((thumb_cid, size))
                }
            })
            .await??;

            self.db
                .new_thumbnail(ThumbnailRow {
                    cid: thumb_cid,
                    size,
                    mimetype: mimetype.to_string(),
                    source_cid: cid.clone(),
                    ratio: f64::from(video_width) / f64::from(video_height),
                    height: preview_height.into(),
                    width: preview_width.into(),
                    is_animated: true,
                })
                .await?;

            // emit event for video preview generated
            self.emit_processing_event(
                cid.clone(),
                ProcessingEventType::VideoPreviewGenerated {
                    long_edge: t_size_long_edge,
                    mimetype: mimetype.to_string(),
                },
                None,
            );
        }
        Ok(())
    }

    pub async fn import_image(
        &self,
        cid: Vec<u8>,
        mimetype: &str,
    ) -> Result<()> {
        let cid_store_path = self.derive_store_path(&cid)?;

        let (decoded_image, exif_data) =
            crate::image::read(&cid_store_path, mimetype)?;

        // extract orientation information from exif data once
        let orientation = exif_data.as_ref().and_then(|exif| {
            exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|orientation| orientation.value.get_uint(0))
        });

        let (img_width, img_height) =
            if orientation == Some(6) || orientation == Some(8) {
                // flipped 90 or 270
                (decoded_image.height(), decoded_image.width())
            } else {
                // not flipped
                (decoded_image.width(), decoded_image.height())
            };

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

        // Clear out old thumbnails as this generates new ones
        self.db.delete_old_thumbnails(cid.clone()).await?;

        // Thumbnail sizes to generate
        let t_sizes_long_edge = vec![320, 640, 1280];

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

            let (thumb_height, thumb_width) = {
                let _permit = self.cpu_semaphore.acquire().await?;
                tokio::task::spawn_blocking({
                    let decoded_image = decoded_image.clone();
                    let thumb_store_path = thumb_store_path.clone();
                    move || {
                        crate::image::thumbnail(
                            &decoded_image,
                            orientation,
                            &thumb_store_path,
                            t_size_long_edge,
                        )
                    }
                })
                .await??
            };

            let (thumb_cid, size) = tokio::task::spawn_blocking({
                let thumb_store_path = thumb_store_path.clone();
                move || -> Result<(Vec<u8>, i64)> {
                    let fh = std::fs::File::open(thumb_store_path)?;
                    let size = fh.metadata()?.len().try_into().unwrap(); // TODO

                    let chunks = crate::ChunkedReader::new(fh);
                    let mut sha_context = crate::cid::new_digest_context();

                    for c in chunks {
                        sha_context.update(&c?);
                    }

                    let thumb_cid =
                        crate::cid::wrap_digest(sha_context.finish())?;
                    Ok((thumb_cid, size))
                }
            })
            .await??;

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

            // emit event for thumbnail generated
            self.emit_processing_event(
                cid.clone(),
                ProcessingEventType::ThumbnailGenerated {
                    long_edge: t_size_long_edge,
                    mimetype: mimetype.to_string(),
                },
                None,
            );
        }

        Ok(())
    }

    pub async fn ext_file_info(
        &self,
        cid: Vec<u8>,
        mimetype: &str,
    ) -> Result<Option<crate::proto::file::ExtFile>> {
        let ret = if mimetype.starts_with("image") {
            let image_row = self.db.image_row(cid.clone()).await?;

            let colors: Vec<Vec<u8>> =
                image_row.colors.chunks(3).map(|s| s.into()).collect();
            let thumbnails = self
                .db
                .thumbnails_by_source_cid(cid)
                .await?
                .iter()
                .map(|t| Thumbnail {
                    cid: t.cid.clone(),
                    size: t.size,
                    mimetype: t.mimetype.clone(),
                    source_cid: t.source_cid.clone(),
                    height: t.height,
                    width: t.width,
                    aspect_ratio: t.ratio as f32,
                    is_animated: t.is_animated,
                })
                .collect();

            Some(crate::proto::file::ExtFile::Image(crate::proto::Image {
                height: image_row.height.into(),
                width: image_row.width.into(),
                aspect_ratio: image_row.ratio as f32,
                colors,
                thumbnails,
            }))
        } else if mimetype.starts_with("video") {
            match self.db.video_row(cid.clone()).await {
                Ok(video_row) => {
                    let thumbnails = self
                        .db
                        .thumbnails_by_source_cid(cid.clone())
                        .await?
                        .iter()
                        .map(|t| Thumbnail {
                            cid: t.cid.clone(),
                            size: t.size,
                            mimetype: t.mimetype.clone(),
                            source_cid: t.source_cid.clone(),
                            height: t.height,
                            width: t.width,
                            aspect_ratio: t.ratio as f32,
                            is_animated: t.is_animated,
                        })
                        .collect();

                    Some(crate::proto::file::ExtFile::Video(
                        crate::proto::Video {
                            height: video_row.height.into(),
                            width: video_row.width.into(),
                            aspect_ratio: video_row.ratio as f32,
                            duration: video_row.duration as f32,
                            thumbnails,
                        },
                    ))
                }
                Err(e) => {
                    let cid_hex = hex::encode(&cid);
                    tracing::warn!("video processing failed for cid={}, returning basic file info: {}", cid_hex, e);
                    // return None instead of propagating error - file still exists, just no extended info
                    None
                }
            }
        } else {
            None
        };

        Ok(ret)
    }

    /// Get the full node ID derived from the node keypair
    pub fn node_id(&self) -> String {
        keys::derive_node_id(&self.node_keypair)
    }

    /// Get the node public key as hex string
    pub fn node_pubkey_hex(&self) -> String {
        keys::keypair_pubkey_to_hex(&self.node_keypair)
    }

    /// Get the consensus public key as hex string (if available)
    pub fn consensus_pubkey_hex(&self) -> Option<String> {
        self.consensus_keypair
            .as_ref()
            .map(keys::keypair_pubkey_to_hex)
    }

    /// Get the instance name from configuration
    pub fn instance_name(&self) -> &str {
        &self.config.instance.name
    }

    /// Get the operator name from configuration
    pub fn operator_name(&self) -> &str {
        &self.config.instance.operator
    }

    pub fn move_to_forgotten(&self, cid: &[u8]) -> Result<()> {
        let current_path = self.derive_store_path(cid)?;

        if !current_path.exists() {
            return Err(anyhow::anyhow!("File does not exist"));
        }

        let forgotten_path = self.derive_forgotten_path(cid)?;

        // ensure forgotten directory exists
        if let Some(parent) = forgotten_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // move file to forgotten directory
        std::fs::rename(&current_path, &forgotten_path)?;

        Ok(())
    }

    pub fn derive_forgotten_path(&self, cid: &[u8]) -> Result<PathBuf> {
        let encoded_cid = crate::cid::encode(cid);

        if encoded_cid.is_empty() {
            return Err(anyhow::anyhow!("Unable to derive path for empty CID"));
        }

        let prefix = if encoded_cid.len() >= 11 {
            &encoded_cid[..11]
        } else {
            &encoded_cid
        };

        let final_dir = self.filestore_path.join("forgotten").join(prefix);
        Ok(final_dir.join(encoded_cid))
    }

    pub async fn forget_file(&self, cid: Vec<u8>) -> Result<()> {
        // get current file path
        let current_path = self.derive_store_path(&cid)?;

        // check if file exists
        if !current_path.exists() {
            return Err(anyhow::anyhow!("File does not exist"));
        }

        // derive forgotten path
        let forgotten_path = self.derive_forgotten_path(&cid)?;

        // ensure forgotten directory exists
        if let Some(parent) = forgotten_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // move file to forgotten directory
        std::fs::rename(&current_path, &forgotten_path)?;

        // clean up thumbnails from filesystem
        for size in [1280, 640, 320, 160] {
            let thumb_path = self.derive_thumb_path(&cid, size)?;
            if thumb_path.exists() {
                let _ = std::fs::remove_file(thumb_path);
            }
        }

        // delete from database (cascades to all related tables)
        self.db.delete_file(cid).await?;

        Ok(())
    }

    pub async fn send_outgoing_message(
        &self,
        msg: crate::mesh_network::OutgoingMessage,
    ) -> Result<()> {
        // extract fields before sending
        let (channel, content, node_id) = match &msg {
            crate::mesh_network::OutgoingMessage::Chat {
                channel,
                content,
                signature: _,
                pubkey: _,
            } => {
                // validate channel name
                let _log_path = self.path_within_filestore(
                    self.filestore_path
                        .join("chat_history")
                        .join(format!("{channel}.log")),
                )?;

                (channel.clone(), content.clone(), self.node_id())
            }
        };

        // send message to mesh network
        if let Err(e) = self.mesh_tx.send(msg).await {
            eprintln!("Failed to send message to mesh network: {e}");
            return Err(anyhow::anyhow!(
                "Failed to send message to mesh network: {}",
                e
            ));
        }

        // log locally
        if let Err(e) =
            self.log_chat_message(&channel, &node_id, &content).await
        {
            eprintln!("Failed to log chat message locally: {e}");
            // don't return error as message was sent successfully
        }

        Ok(())
    }

    async fn log_chat_message(
        &self,
        channel: &str,
        sender: &str,
        content: &str,
    ) -> Result<()> {
        self.chatroom.log_message(channel, sender, content).await
    }

    pub fn sanitize_filestore_path(
        &self,
        subdirectory: &str,
        filename: &str,
    ) -> Result<PathBuf> {
        sanitize_filestore_path(&self.filestore_path, subdirectory, filename)
    }

    fn path_within_filestore(&self, path: PathBuf) -> Result<PathBuf> {
        let canonical_filestore = self
            .filestore_path
            .canonicalize()
            .unwrap_or_else(|_| self.filestore_path.clone());
        let canonical_path =
            path.canonicalize().unwrap_or_else(|_| path.clone());

        if canonical_path.starts_with(&canonical_filestore) {
            Ok(path)
        } else {
            Err(anyhow::anyhow!("path is outside filestore directory"))
        }
    }

    pub async fn get_chat_history(
        &self,
        channel: &str,
        page_token: &str,
        page_size: u32,
    ) -> Result<(Vec<crate::proto::ChatMessageInfo>, String)> {
        let log_file = self.path_within_filestore(
            self.filestore_path
                .join("chat_history")
                .join(format!("{channel}.log")),
        )?;

        if !log_file.exists() {
            return Ok((vec![], String::new()));
        }

        let file = std::fs::File::open(&log_file)?;
        let rev_lines = rev_lines::RevLines::new(file);

        let start_line: usize = if page_token.is_empty() {
            0
        } else {
            page_token.parse().unwrap_or(0)
        };

        let mut rev_iter = rev_lines.skip(start_line);
        let mut collected_lines = Vec::new();

        // collect up to page_size lines
        for _ in 0..page_size {
            match rev_iter.next() {
                Some(Ok(line)) => collected_lines.push(line),
                Some(Err(e)) => return Err(e.into()),
                None => break, // no more lines
            }
        }

        // check if there are more lines for next_page_token
        let has_more = rev_iter.next().is_some();

        let messages: Vec<crate::proto::ChatMessageInfo> = collected_lines
            .into_iter()
            .rev()
            .map(|line| crate::proto::ChatMessageInfo {
                channel: channel.to_string(),
                content: line,
                node_id: "".to_string(),
                signature: vec![],
            })
            .collect();

        let next_page_token = if has_more {
            (start_line + messages.len()).to_string()
        } else {
            String::new()
        };

        Ok((messages, next_page_token))
    }

    pub async fn get_chat_channels(
        &self,
    ) -> Result<Vec<crate::proto::ChatChannelInfo>> {
        Ok(vec![
            crate::proto::ChatChannelInfo {
                name: "general".to_string(),
            },
            crate::proto::ChatChannelInfo {
                name: "dev".to_string(),
            },
        ])
    }
}
