use anyhow::Result;
use async_trait::async_trait;

use crate::proto::{File, Tag};

#[derive(Hash, Eq, PartialEq, Clone)]
pub struct TagRow {
    pub id: i32,
    pub namespace: String,
    pub descriptor: String,
}

#[derive(Debug)]
pub struct FileRow {
    pub cid: Vec<u8>,
    pub size: i64,
    pub mimetype: Option<String>,
}

pub struct TagMapRow {
    pub file_cid: Vec<u8>,
    pub tag_id: i32,
    pub added: Option<String>,
    pub reason: u32,
}

pub struct ImageRow {
    pub cid: Vec<u8>,
    pub height: u32,
    pub width: u32,
    pub ratio: f64,
    pub primary_color: Vec<u8>,
    pub colors: Vec<u8>,
}

pub struct VideoRow {
    pub cid: Vec<u8>,
    pub height: u32,
    pub width: u32,
    pub ratio: f64,
    pub duration: f64,
}

pub struct ThumbnailRow {
    pub cid: Vec<u8>,
    pub size: i64,
    pub mimetype: String,
    pub source_cid: Vec<u8>,
    pub height: i64,
    pub width: i64,
    pub ratio: f64,
    pub is_animated: bool,
}

#[derive(Debug, Clone)]
pub struct TagRowCount {
    pub id: i32,
    pub namespace: String,
    pub descriptor: String,
    pub count: i64,
}

#[async_trait]
pub trait DatabaseBackend: Send + Sync {
    async fn init_tables(&mut self) -> Result<()>;
    async fn file_tags(&self, cid: Vec<u8>) -> Result<Vec<TagRow>>;
    async fn new_file(&self, f: FileRow) -> Result<()>;
    async fn new_tag_vocab(&self, tags: Vec<Tag>) -> Result<()>;
    async fn new_tag_map(&self, tag_maps: &[TagMapRow]) -> Result<()>;
    async fn remove_tag_map(
        &self,
        file_cid: Vec<u8>,
        tag_ids: &[i32],
    ) -> Result<()>;
    async fn new_thumbnail(&self, thumbnail: ThumbnailRow) -> Result<()>;
    async fn delete_old_thumbnails(&self, cid: Vec<u8>) -> Result<()>;
    async fn new_image(&self, image: ImageRow) -> Result<()>;
    async fn new_video(&self, video: VideoRow) -> Result<()>;
    async fn lookup_tag_id(&self, tags: Vec<Tag>) -> Result<Vec<TagRow>>;
    async fn file_row(&self, cid: Vec<u8>) -> Result<FileRow>;
    async fn image_row(&self, cid: Vec<u8>) -> Result<ImageRow>;
    async fn video_row(&self, cid: Vec<u8>) -> Result<VideoRow>;
    async fn thumbnails_by_source_cid(
        &self,
        cid: Vec<u8>,
    ) -> Result<Vec<ThumbnailRow>>;
    async fn file_page(
        &self,
        count: u32,
        offset: u32,
        oldest_first: bool,
    ) -> Result<Vec<FileRow>>;
    async fn files_page(
        &self,
        query: Option<crate::proto::SearchQuery>,
        page_size: u32,
        page_number: u32,
        sort_order: i32,
        reverse_order: bool,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<(Vec<File>, u32)>;
    async fn tags_page(
        &self,
        page_size: u32,
        page_number: u32,
        sort_order: i32,
        reverse_order: bool,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<(Vec<TagRowCount>, u32)>;
    async fn get_most_popular_tags(
        &self,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<Vec<TagRowCount>>;
    async fn get_most_popular_tags_within_namespace_that_starts_with(
        &self,
        tag_constraints: &[crate::proto::TagQuery],
        begins_with: &str,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<Vec<TagRowCount>>;
    async fn get_descriptors_that_start_with(
        &self,
        tag_constraints: &[crate::proto::TagQuery],
        namespace: Option<String>,
        begins_with: &str,
        visibility_filter: crate::visibility::VisibilityFilter,
    ) -> Result<Vec<TagRowCount>>;
    async fn random_file(&self, count: u32) -> Result<Vec<FileRow>>;
    async fn count_files(&self) -> Result<i64>;
    async fn count_tags(&self) -> Result<i64>;
    async fn count_tag_associations(&self) -> Result<i64>;
    async fn delete_file(&self, cid: Vec<u8>) -> Result<()>;
    async fn batch_new_tag_maps(
        &self,
        all_tag_maps: &[TagMapRow],
    ) -> Result<()>;
    async fn batch_remove_tag_maps(
        &self,
        removals: &[(Vec<u8>, i32)],
    ) -> Result<()>;
}

pub async fn fetch_thumbnails_for<T: DatabaseBackend>(
    db: &T,
    source_cid: &[u8],
) -> Result<Vec<crate::proto::Thumbnail>> {
    let thumbnails = db.thumbnails_by_source_cid(source_cid.to_vec()).await?;
    Ok(thumbnails
        .into_iter()
        .map(|t| crate::proto::Thumbnail {
            cid: t.cid,
            size: t.size,
            mimetype: t.mimetype,
            source_cid: t.source_cid,
            height: t.height,
            width: t.width,
            aspect_ratio: t.ratio as f32,
            is_animated: t.is_animated,
        })
        .collect())
}
