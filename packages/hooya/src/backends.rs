use anyhow::Result;
use async_trait::async_trait;

use crate::local::{
    DatabaseBackend, FileRow, ImageRow, TagMapRow, TagRow, TagRowCount,
    ThumbnailRow, VideoRow,
};
use crate::proto::{File, Tag};

// SQLite Backend Implementation
pub mod sqlite {
    use super::*;
    use sqlx::{
        sqlite::{SqlitePool, SqliteRow},
        Executor, QueryBuilder, Row, Sqlite,
    };

    pub struct SqliteBackend {
        pub executor: SqlitePool,
    }

    #[async_trait]
    impl DatabaseBackend for SqliteBackend {
        async fn init_tables(&mut self) -> Result<()> {
            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Files(
                Cid VARBINARY NOT NULL PRIMARY KEY,
                Size UNSIGNED BIGINT,
                Mimetype TEXT,
                Indexed DATETIME DEFAULT CURRENT_TIMESTAMP)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Tags (
                Id INTEGER PRIMARY KEY AUTOINCREMENT,
                Namespace TEXT,
                Descriptor TEXT NOT NULL,
                UNIQUE(Namespace, Descriptor))"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS TagMap (
                FileCid VARBINARY NOT NULL,
                TagId INTEGER NOT NULL,
                Added DATETIME DEFAULT CURRENT_TIMESTAMP,
                Reason INTEGER UNSIGNED NOT NULL,
                UNIQUE(FileCid, TagId),
                FOREIGN KEY (FileCid) REFERENCES Files(Cid) ON DELETE CASCADE,
                FOREIGN KEY (TagId) REFERENCES Tags(Id) ON DELETE CASCADE)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Images (
                Cid VARBINARY NOT NULL PRIMARY KEY,
                Height INTEGER UNSIGNED NOT NULL,
                Width INTEGER UNSIGNED NOT NULL,
                Ratio REAL NOT NULL,
                PrimaryColor BINARY(3) DEFAULT NULL,
                Colors VARBINARY DEFAULT NULL,
                FOREIGN KEY (Cid) REFERENCES Files(Cid) ON DELETE CASCADE)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Videos (
                Cid VARBINARY NOT NULL PRIMARY KEY,
                Height INTEGER UNSIGNED NOT NULL,
                Width INTEGER UNSIGNED NOT NULL,
                Ratio REAL NOT NULL,
                Duration FLOAT NOT NULL,
                FOREIGN KEY (Cid) REFERENCES Files(Cid) ON DELETE CASCADE)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Thumbnails (
                Cid VARBINARY NOT NULL PRIMARY KEY,
                Size UNSIGNED BIGINT,
                Mimetype TEXT,
                SourceCid VARBINARY NOT NULL,
                Height INTEGER UNSIGNED NOT NULL,
                Width INTEGER UNSIGNED NOT NULL,
                Ratio REAL NOT NULL,
                IsAnimated BOOLEAN DEFAULT FALSE NOT NULL,
                FOREIGN KEY (SourceCid) REFERENCES Files(Cid) ON DELETE CASCADE)"#,
                )
                .await?;

            Ok(())
        }

        async fn file_tags(&self, cid: Vec<u8>) -> Result<Vec<TagRow>> {
            let tag_rows = sqlx::query(
                "SELECT Id, Namespace, Descriptor FROM Tags, TagMap WHERE
                FileCid = ? AND TagId = Id",
            )
            .bind(cid)
            .try_map(|r: SqliteRow| {
                let id = r.try_get("Id")?;
                let namespace = r.try_get("Namespace")?;
                let descriptor = r.try_get("Descriptor")?;

                Ok(TagRow {
                    id,
                    namespace,
                    descriptor,
                })
            })
            .fetch_all(&self.executor)
            .await?;

            Ok(tag_rows)
        }

        async fn new_file(&self, f: FileRow) -> Result<()> {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO Files (Cid, Size, Mimetype) VALUES
                (?, ?, ?)"#,
            )
            .bind(f.cid)
            .bind(f.size)
            .bind(f.mimetype)
            .execute(&self.executor)
            .await?;

            Ok(())
        }

        async fn new_tag_vocab(&self, tags: Vec<Tag>) -> Result<()> {
            if tags.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for t in tags {
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO Tags (Namespace, Descriptor) VALUES
                    (?, ?)"#,
                )
                .bind(t.namespace)
                .bind(t.descriptor)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn new_tag_map(&self, tag_maps: &[TagMapRow]) -> Result<()> {
            if tag_maps.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for t in tag_maps {
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO TagMap (FileCid, TagId, Added, Reason) VALUES
                    (?, ?, ?, ?)"#,
                )
                .bind(t.file_cid.clone())
                .bind(t.tag_id)
                .bind(t.added.clone())
                .bind(t.reason as i32)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn remove_tag_map(
            &self,
            file_cid: Vec<u8>,
            tag_ids: &[i32],
        ) -> Result<()> {
            if tag_ids.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for tag_id in tag_ids {
                sqlx::query(
                    r#"
                    DELETE FROM TagMap WHERE FileCid = ? AND TagId = ?"#,
                )
                .bind(&file_cid)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn new_thumbnail(&self, thumbnail: ThumbnailRow) -> Result<()> {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO Thumbnails (Cid, Size, Mimetype, SourceCid, Height, Width, Ratio, IsAnimated) VALUES
                (?, ?, ?, ?, ?, ?, ?, ?)"#,
            )
            .bind(thumbnail.cid)
            .bind(thumbnail.size)
            .bind(thumbnail.mimetype)
            .bind(thumbnail.source_cid)
            .bind(thumbnail.height)
            .bind(thumbnail.width)
            .bind(thumbnail.ratio)
            .bind(thumbnail.is_animated)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn delete_old_thumbnails(&self, cid: Vec<u8>) -> Result<()> {
            sqlx::query(
                r#"
                DELETE FROM Thumbnails WHERE SourceCid=?"#,
            )
            .bind(cid)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn new_image(&self, image: ImageRow) -> Result<()> {
            sqlx::query(
                r#"
                INSERT INTO Images (Cid, Height, Width, Ratio, PrimaryColor, Colors) VALUES
                (?, ?, ?, ?, ?, ?) ON CONFLICT(Cid)
                    DO UPDATE SET
                    Height=excluded.Height, Width=excluded.Width,
                    Ratio=excluded.Ratio, PrimaryColor=excluded.PrimaryColor,
                    Colors=excluded.Colors"#,
            )
            .bind(image.cid)
            .bind(image.height as i32)
            .bind(image.width as i32)
            .bind(image.ratio)
            .bind(image.primary_color)
            .bind(image.colors)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn new_video(&self, video: VideoRow) -> Result<()> {
            sqlx::query(
                r#"
                INSERT INTO Videos (Cid, Height, Width, Ratio, Duration) VALUES
                (?, ?, ?, ?, ?) ON CONFLICT(Cid)
                    DO UPDATE SET
                    Height=excluded.Height, Width=excluded.Width,
                    Ratio=excluded.Ratio, Duration=excluded.Duration"#,
            )
            .bind(video.cid)
            .bind(video.height as i32)
            .bind(video.width as i32)
            .bind(video.ratio)
            .bind(video.duration)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn lookup_tag_id(&self, tags: Vec<Tag>) -> Result<Vec<TagRow>> {
            if tags.is_empty() {
                return Ok(vec![]);
            }

            let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new(
                r#"SELECT Id, Descriptor, Namespace
            FROM Tags WHERE (Namespace, Descriptor) IN ("#,
            );
            let tags_len = tags.len();
            for (i, t) in tags.into_iter().enumerate() {
                builder.push("(");
                builder.push_bind(t.namespace);
                builder.push(",");
                builder.push_bind(t.descriptor);
                builder.push(")");
                if i < tags_len - 1 {
                    builder.push(", ");
                }
            }
            builder.push(")");
            let query = builder.build();

            let tag_rows = query
                .try_map(|r: SqliteRow| {
                    let descriptor = r.try_get("Descriptor")?;
                    let namespace = r.try_get("Namespace")?;
                    let id = r.try_get("Id")?;

                    Ok(TagRow {
                        id,
                        descriptor,
                        namespace,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(tag_rows)
        }

        async fn file_row(&self, cid: Vec<u8>) -> Result<FileRow> {
            let file_row = sqlx::query(
                "SELECT Cid, Mimetype, Size FROM Files WHERE Cid=?",
            )
            .bind(cid)
            .try_map(|r: SqliteRow| {
                let cid = r.try_get("Cid")?;
                let mimetype = r.try_get("Mimetype")?;
                let size = r.try_get("Size")?;

                Ok(FileRow {
                    cid,
                    mimetype,
                    size,
                })
            })
            .fetch_one(&self.executor)
            .await?;

            Ok(file_row)
        }

        async fn image_row(&self, cid: Vec<u8>) -> Result<ImageRow> {
            let row =
                sqlx::query("SELECT Cid, Height, Width, Ratio, PrimaryColor, Colors FROM Images WHERE Cid=?")
                    .bind(cid)
                    .try_map(|r: SqliteRow| {
                        let cid = r.try_get("Cid")?;
                        let height = r.try_get::<i32, _>("Height")? as u32;
                        let width = r.try_get::<i32, _>("Width")? as u32;
                        let ratio = r.try_get("Ratio")?;
                        let primary_color = r.try_get("PrimaryColor")?;
                        let colors = r.try_get("Colors")?;

                        Ok(ImageRow {
                            cid,
                            height,
                            width,
                            ratio,
                            primary_color,
                            colors,
                        })
                    })
                    .fetch_one(&self.executor)
                    .await?;

            Ok(row)
        }

        async fn video_row(&self, cid: Vec<u8>) -> Result<VideoRow> {
            let row =
                sqlx::query("SELECT Cid, Height, Width, Ratio, Duration FROM Videos WHERE Cid=?")
                    .bind(cid)
                    .try_map(|r: SqliteRow| {
                        let cid = r.try_get("Cid")?;
                        let height = r.try_get::<i32, _>("Height")? as u32;
                        let width = r.try_get::<i32, _>("Width")? as u32;
                        let ratio = r.try_get("Ratio")?;
                        let duration = r.try_get("Duration")?;

                        Ok(VideoRow {
                            cid,
                            height,
                            width,
                            ratio,
                            duration,
                        })
                    })
                    .fetch_one(&self.executor)
                    .await?;

            Ok(row)
        }

        async fn thumbnails_by_source_cid(
            &self,
            cid: Vec<u8>,
        ) -> Result<Vec<ThumbnailRow>> {
            let thumbnail_rows = sqlx::query("SELECT Cid, Size, Mimetype, SourceCid, Height, Width, Ratio, IsAnimated FROM Thumbnails WHERE SourceCid=?")
                .bind(cid)
                .try_map(|r: SqliteRow| {
                    let cid = r.try_get("Cid")?;
                    let size = r.try_get("Size")?;
                    let mimetype = r.try_get("Mimetype")?;
                    let source_cid = r.try_get("SourceCid")?;
                    let height = r.try_get::<i64, _>("Height")?;
                    let width = r.try_get::<i64, _>("Width")?;
                    let ratio = r.try_get("Ratio")?;
                    let is_animated = r.try_get("IsAnimated")?;

                    Ok(ThumbnailRow {
                        cid,
                        size,
                        mimetype,
                        source_cid,
                        height,
                        width,
                        ratio,
                        is_animated,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(thumbnail_rows)
        }

        async fn file_page(
            &self,
            count: u32,
            offset: u32,
            oldest_first: bool,
        ) -> Result<Vec<FileRow>> {
            let query = if oldest_first {
                "SELECT Cid, Mimetype, Size FROM Files ORDER BY Indexed LIMIT ? OFFSET ?"
            } else {
                "SELECT Cid, Mimetype, Size FROM Files ORDER BY Indexed DESC LIMIT ? OFFSET ?"
            };
            let file_rows = sqlx::query(query)
                .bind(count)
                .bind(offset)
                .try_map(|r: SqliteRow| {
                    let cid = r.try_get("Cid")?;
                    let mimetype = r.try_get("Mimetype")?;
                    let size = r.try_get("Size")?;

                    Ok(FileRow {
                        cid,
                        mimetype,
                        size,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(file_rows)
        }

        async fn files_page(
            &self,
            query: Option<crate::proto::SearchQuery>,
            page_size: u32,
            page_number: u32,
            sort_order: i32,
            reverse_order: bool,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<(Vec<File>, u32)> {
            let offset = (page_number.saturating_sub(1)) * page_size;

            let order_clause = match (sort_order, reverse_order) {
                (0, true) => "ORDER BY Indexed",
                (0, false) => "ORDER BY Indexed DESC",
                _ => return Err(anyhow::anyhow!("Invalid sort order")),
            };

            let sql_query;
            let count_sql_query;
            let prepared_statement = if let Some(q) = query {
                let mut where_clause = String::new();
                if !q.tag_query.is_empty() {
                    for (i, q) in q.tag_query.iter().enumerate() {
                        if i > 0 {
                            where_clause.push_str(" OR ");
                        }
                        if q.negated {
                            where_clause.push_str(" NOT ")
                        }
                        where_clause
                            .push_str("(t.namespace = ? AND t.descriptor = ?)");
                    }
                }

                if !where_clause.is_empty() {
                    where_clause.push_str(" AND ");
                }

                let mut visibility_conditions = Vec::new();

                if visibility_filter.includes_public() {
                    visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
                }

                if visibility_filter.includes_unindexed() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
                }

                if visibility_filter.includes_private() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
                }

                if visibility_conditions.is_empty() {
                    where_clause.push_str("1 = 0"); // no visibility specified
                } else {
                    where_clause.push_str(&format!(
                        "({})",
                        visibility_conditions.join(" OR ")
                    ));
                }

                let distinct_tag_count = q.tag_query.len();
                sql_query = format!(
                    r#"
                    SELECT
                        f.cid,
                        f.Size,
                        f.Mimetype,
                        i.Height as "ImageHeight?",
                        i.Width as "ImageWidth?",
                        i.Ratio as "ImageRatio?",
                        i.Colors as "ImageColors?",
                        i.PrimaryColor as "ImagePrimaryColor?",
                        v.Height as "VideoHeight?",
                        v.Width as "VideoWidth?",
                        v.Ratio as "VideoRatio?",
                        v.Duration as "VideoDuration?"
                    FROM Files f
                    LEFT JOIN Images i ON f.cid = i.Cid
                    LEFT JOIN Videos v ON f.cid = v.Cid
                    INNER JOIN TagMap tm ON f.cid = tm.filecid
                    INNER JOIN Tags t ON t.id = tm.tagid
                    WHERE {where_clause}
                    GROUP BY f.cid
                    HAVING COUNT(DISTINCT t.id) = {distinct_tag_count}
                    {order_clause}
                    LIMIT ? OFFSET ?
                "#
                );

                count_sql_query = format!(
                    r#"
                    SELECT COUNT(DISTINCT f.cid) as total
                    FROM Files f
                    LEFT JOIN Images i ON f.cid = i.Cid
                    LEFT JOIN Videos v ON f.cid = v.Cid
                    INNER JOIN TagMap tm ON f.cid = tm.filecid
                    INNER JOIN Tags t ON t.id = tm.tagid
                    WHERE {where_clause}
                "#
                );

                // Bind namespace:descriptor parameters defined earlier
                let mut query = sqlx::query(&sql_query);
                let mut count_query = sqlx::query(&count_sql_query);
                for tag in q.tag_query {
                    let namespace = tag
                        .namespace
                        .unwrap_or("general".to_string())
                        .to_ascii_lowercase();
                    let descriptor = tag.descriptor.to_ascii_lowercase();

                    query =
                        query.bind(namespace.clone()).bind(descriptor.clone());
                    count_query = count_query.bind(namespace).bind(descriptor);
                }

                // Lastly bind the pagination parameters
                query = query.bind(page_size).bind(offset);

                (query, count_query)
            } else {
                let mut visibility_conditions = Vec::new();

                if visibility_filter.includes_public() {
                    visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
                }

                if visibility_filter.includes_unindexed() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
                }

                if visibility_filter.includes_private() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
                }

                let visibility_where = if visibility_conditions.is_empty() {
                    "WHERE 1 = 0" // no visibility types allowed
                } else {
                    &format!("WHERE ({})", visibility_conditions.join(" OR "))
                };

                sql_query = format!(
                    r#"
                    SELECT
                        f.cid,
                        f.Size,
                        f.Mimetype,
                        i.Height as "ImageHeight?",
                        i.Width as "ImageWidth?",
                        i.Ratio as "ImageRatio?",
                        i.Colors as "ImageColors?",
                        i.PrimaryColor as "ImagePrimaryColor?",
                        v.Height as "VideoHeight?",
                        v.Width as "VideoWidth?",
                        v.Ratio as "VideoRatio?",
                        v.Duration as "VideoDuration?"
                    FROM Files f
                    LEFT JOIN Images i ON f.cid = i.Cid
                    LEFT JOIN Videos v ON f.cid = v.Cid
                    {visibility_where}
                    {order_clause}
                    LIMIT ? OFFSET ?
                "#
                );
                count_sql_query = format!(
                    r#"SELECT COUNT(*) as total FROM files f {visibility_where}"#
                );

                let query =
                    sqlx::query(&sql_query).bind(page_size).bind(offset);
                let count_query = sqlx::query(&count_sql_query);
                (query, count_query)
            };

            let raw_files =
                prepared_statement.0.fetch_all(&self.executor).await?;

            let total_count: u32 = prepared_statement
                .1
                .fetch_one(&self.executor)
                .await?
                .try_get("total")?;
            let final_page_token = if total_count % page_size == 0 {
                total_count / page_size
            } else {
                (total_count / page_size) + 1
            };

            let mut files: Vec<File> = Vec::with_capacity(raw_files.len());
            for raw_file in raw_files.into_iter() {
                let cid: Vec<u8> = raw_file.try_get("Cid")?;
                let thumbnails =
                    crate::local::fetch_thumbnails_for(self, &cid).await?;

                // fetch tags for this file using existing method
                let tag_rows = self.file_tags(cid.clone()).await?;
                let tags = tag_rows
                    .into_iter()
                    .map(|tag_row| Tag {
                        namespace: tag_row.namespace,
                        descriptor: tag_row.descriptor,
                    })
                    .collect();

                let ext_file = if let Ok(height) =
                    raw_file.try_get("ImageHeight?")
                {
                    let colors = raw_file
                        .try_get::<Vec<u8>, _>("ImageColors?")?
                        .chunks(3)
                        .map(|c| c.into())
                        .collect();
                    Some(crate::proto::file::ExtFile::Image(
                        crate::proto::Image {
                            height,
                            width: raw_file.try_get("ImageWidth?")?,
                            aspect_ratio: raw_file.try_get("ImageRatio?")?,
                            colors,
                            thumbnails,
                        },
                    ))
                } else if let Ok(height) = raw_file.try_get("VideoHeight?") {
                    Some(crate::proto::file::ExtFile::Video(
                        crate::proto::Video {
                            height,
                            width: raw_file.try_get("VideoWidth?")?,
                            aspect_ratio: raw_file.try_get("VideoRatio?")?,
                            thumbnails,
                            duration: raw_file.try_get("VideoDuration?")?,
                        },
                    ))
                } else {
                    None
                };

                files.push(File {
                    cid,
                    size: raw_file.try_get("Size")?,
                    mimetype: raw_file.try_get("Mimetype")?,
                    processing_status: Default::default(),
                    ext_file,
                    tags,
                });
            }

            Ok((files, final_page_token))
        }

        async fn tags_page(
            &self,
            page_size: u32,
            page_number: u32,
            sort_order: i32,
            reverse_order: bool,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<(Vec<TagRowCount>, u32)> {
            let offset = (page_number.saturating_sub(1)) * page_size;

            let order_clause = match (sort_order, reverse_order) {
                (0, true) => "ORDER BY Count",
                (0, false) => "ORDER BY Count DESC",
                _ => return Err(anyhow::anyhow!("Invalid sort order")),
            };

            let total_count_query = r#"
                SELECT COUNT(DISTINCT Id) as total_count
                FROM Tags
            "#;

            let total_count_row = sqlx::query(total_count_query)
                .fetch_one(&self.executor)
                .await?;

            let total_count: u32 = total_count_row.try_get("total_count")?;
            let final_page_token = if page_size > 0 {
                total_count.div_ceil(page_size)
            } else {
                1
            };

            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            let visibility_where = if visibility_conditions.is_empty() {
                "WHERE 1 = 0" // no visibility types allowed
            } else {
                &format!("WHERE ({})", visibility_conditions.join(" OR "))
            };

            let query = format!(
                r#"
                    SELECT
                        t.id,
                        t.namespace,
                        t.descriptor,
                        COUNT(*) as "count"
                    FROM Tags t
                    INNER JOIN TagMap tm ON t.id = tm.tagid
                    INNER JOIN Files f ON f.cid = tm.filecid
                    {visibility_where}
                    GROUP BY t.id
                    {order_clause}
                    LIMIT ? OFFSET ?
                "#
            );

            let prepared_statement = sqlx::query(&query)
                .bind(page_size)
                .bind(offset)
                .try_map(|t: SqliteRow| {
                    Ok(TagRowCount {
                        id: t.try_get("Id")?,
                        namespace: t.try_get("Namespace")?,
                        descriptor: t.try_get("Descriptor")?,
                        count: t.try_get("count")?,
                    })
                });

            let tags_info =
                prepared_statement.fetch_all(&self.executor).await?;

            Ok((tags_info, final_page_token))
        }

        async fn get_most_popular_tags(
            &self,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<Vec<TagRowCount>> {
            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            let visibility_where = if visibility_conditions.is_empty() {
                "WHERE 1 = 0" // no visibility types allowed
            } else {
                &format!("WHERE ({})", visibility_conditions.join(" OR "))
            };

            let query = format!(
                r#"
                SELECT t.id, t.namespace, t.descriptor, COUNT(*) AS associations FROM Tags t
                INNER JOIN TagMap tm ON t.id = tm.tagid
                INNER JOIN Files f ON tm.filecid = f.cid
                {visibility_where}
                GROUP BY t.id
                ORDER BY associations DESC
                LIMIT 10
                "#
            );

            let ret = sqlx::query(&query)
                .try_map(|r: SqliteRow| {
                    let count = r.try_get("associations")?;
                    let descriptor = r.try_get("Descriptor")?;
                    let namespace = r.try_get("Namespace")?;
                    let id = r.try_get("Id")?;
                    Ok(TagRowCount {
                        id,
                        namespace,
                        descriptor,
                        count,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(ret)
        }

        async fn get_most_popular_tags_within_namespace_that_starts_with(
            &self,
            tag_constraints: &[crate::proto::TagQuery],
            begins_with: &str,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<Vec<TagRowCount>> {
            let mut params_bind = Vec::new();

            let mut where_clauses = vec![];

            let like_clause = format!("{begins_with}%");
            params_bind.push(like_clause);
            where_clauses.push("t.namespace LIKE ?".to_string());

            for constraint in tag_constraints {
                let negation_clause =
                    if constraint.negated { "NOT" } else { "" };

                let namespace = constraint
                    .namespace
                    .clone()
                    .unwrap_or_else(|| "general".to_string());

                params_bind.push(namespace);
                params_bind.push(constraint.descriptor.clone());

                let subquery = format!(
                    r#"
                    {negation_clause} EXISTS (
                        SELECT 1 FROM TagMap tm
                        INNER JOIN Tags tg ON tm.tagid = tg.Id
                        WHERE tm.filecid = f.cid AND tg.namespace = ? AND tg.descriptor = ?
                    )"#,
                );
                where_clauses.push(subquery);
            }

            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            if !visibility_conditions.is_empty() {
                where_clauses
                    .push(format!("({})", visibility_conditions.join(" OR ")));
            } else {
                where_clauses.push("1 = 0".to_string()); // no visibility types allowed
            }

            let having_clause = if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                format!(
                    "HAVING COUNT(DISTINCT tm.filecid) >= {non_negated_count}"
                )
            } else {
                String::new()
            };

            let sql_query = format!(
                r#"
                SELECT t.id, t.namespace, t.descriptor, COUNT(DISTINCT tm.filecid) AS associations FROM Tags t
                INNER JOIN TagMap tm ON t.id = tm.tagid
                INNER JOIN Files f ON tm.filecid = f.cid
                WHERE {}
                GROUP BY t.id
                {}
                LIMIT 10
            "#,
                where_clauses.join(" AND "),
                having_clause
            );

            let mut prepared_statement = sqlx::query(&sql_query);
            for param in params_bind {
                prepared_statement = prepared_statement.bind(param);
            }

            if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                prepared_statement =
                    prepared_statement.bind(non_negated_count as i64);
            }

            let res = prepared_statement
                .try_map(|r: SqliteRow| {
                    let count = r.try_get("associations")?;
                    let descriptor = r.try_get("Descriptor")?;
                    let namespace = r.try_get("Namespace")?;
                    let id = r.try_get("Id")?;
                    Ok(TagRowCount {
                        id,
                        count,
                        namespace,
                        descriptor,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(res)
        }

        async fn get_descriptors_that_start_with(
            &self,
            tag_constraints: &[crate::proto::TagQuery],
            namespace: Option<String>,
            begins_with: &str,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<Vec<TagRowCount>> {
            let mut params_bind = Vec::new();

            let mut where_clauses = vec![];

            let like_clause = format!("{begins_with}%");
            params_bind.push(like_clause);
            where_clauses.push("t.descriptor LIKE ?".to_string());

            if let Some(n) = namespace {
                params_bind.push(n);
                where_clauses.push("t.namespace = ?".to_string());
            }

            for constraint in tag_constraints {
                let negation_clause =
                    if constraint.negated { "NOT" } else { "" };

                let namespace = constraint
                    .namespace
                    .clone()
                    .unwrap_or_else(|| "general".to_string());

                params_bind.push(namespace);
                params_bind.push(constraint.descriptor.clone());

                let subquery = format!(
                    r#"
                    {negation_clause} EXISTS (
                        SELECT 1 FROM TagMap tm
                        INNER JOIN Tags tg ON tm.tagid = tg.Id
                        WHERE tm.filecid = f.cid AND tg.namespace = ? AND tg.descriptor = ?
                    )"#,
                );
                where_clauses.push(subquery);
            }

            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            if !visibility_conditions.is_empty() {
                where_clauses
                    .push(format!("({})", visibility_conditions.join(" OR ")));
            } else {
                where_clauses.push("1 = 0".to_string()); // no visibility types allowed
            }

            let having_clause = if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                format!(
                    "HAVING COUNT(DISTINCT tm.filecid) >= {non_negated_count}"
                )
            } else {
                String::new()
            };

            let sql_query = format!(
                r#"
                SELECT t.id, t.namespace, t.descriptor, COUNT(DISTINCT tm.filecid) AS associations FROM Tags t
                INNER JOIN TagMap tm ON t.id = tm.tagid
                INNER JOIN Files f ON tm.filecid = f.cid
                WHERE {}
                GROUP BY t.id
                {}
                LIMIT 10
            "#,
                where_clauses.join(" AND "),
                having_clause
            );

            let mut prepared_statement = sqlx::query(&sql_query);
            for param in params_bind {
                prepared_statement = prepared_statement.bind(param);
            }

            if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                prepared_statement =
                    prepared_statement.bind(non_negated_count as i64);
            }

            let res = prepared_statement
                .try_map(|r: SqliteRow| {
                    let count = r.try_get("associations")?;
                    let descriptor = r.try_get("Descriptor")?;
                    let namespace = r.try_get("Namespace")?;
                    let id = r.try_get("Id")?;
                    Ok(TagRowCount {
                        id,
                        count,
                        namespace,
                        descriptor,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(res)
        }

        async fn random_file(&self, count: u32) -> Result<Vec<FileRow>> {
            let file_rows = sqlx::query(
                "SELECT Cid, Mimetype, Size FROM Files ORDER BY RANDOM() LIMIT ?",
            )
            .bind(count)
            .try_map(|r: SqliteRow| {
                let cid = r.try_get("Cid")?;
                let mimetype = r.try_get("Mimetype")?;
                let size = r.try_get("Size")?;

                Ok(FileRow {
                    cid,
                    mimetype,
                    size,
                })
            })
            .fetch_all(&self.executor)
            .await?;

            Ok(file_rows)
        }

        async fn count_files(&self) -> Result<i64> {
            let row = sqlx::query("SELECT COUNT(*) as count FROM Files")
                .fetch_one(&self.executor)
                .await?;
            Ok(row.get("count"))
        }

        async fn count_tags(&self) -> Result<i64> {
            let row = sqlx::query("SELECT COUNT(*) as count FROM Tags")
                .fetch_one(&self.executor)
                .await?;
            Ok(row.get("count"))
        }

        async fn count_tag_associations(&self) -> Result<i64> {
            let row = sqlx::query("SELECT COUNT(*) as count FROM TagMap")
                .fetch_one(&self.executor)
                .await?;
            Ok(row.get("count"))
        }

        async fn delete_file(&self, cid: Vec<u8>) -> Result<()> {
            sqlx::query("DELETE FROM Files WHERE Cid = ?")
                .bind(cid)
                .execute(&self.executor)
                .await?;
            Ok(())
        }

        async fn batch_new_tag_maps(
            &self,
            all_tag_maps: &[TagMapRow],
        ) -> Result<()> {
            if all_tag_maps.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for t in all_tag_maps {
                sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO TagMap (FileCid, TagId, Added, Reason) VALUES
                    (?, ?, ?, ?)"#,
                )
                .bind(t.file_cid.clone())
                .bind(t.tag_id)
                .bind(t.added.clone())
                .bind(t.reason as i32)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn batch_remove_tag_maps(
            &self,
            removals: &[(Vec<u8>, i32)],
        ) -> Result<()> {
            if removals.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for (file_cid, tag_id) in removals {
                sqlx::query(
                    r#"
                    DELETE FROM TagMap WHERE FileCid = ? AND TagId = ?"#,
                )
                .bind(file_cid)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }
    }
}

// PostgreSQL Backend Implementation
pub mod postgres {
    use super::*;
    use sqlx::{
        postgres::{PgPool, PgRow},
        Executor, Postgres, QueryBuilder, Row,
    };

    pub struct PostgresBackend {
        pub executor: PgPool,
    }

    #[async_trait]
    impl DatabaseBackend for PostgresBackend {
        async fn init_tables(&mut self) -> Result<()> {
            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Files(
                cid BYTEA NOT NULL PRIMARY KEY,
                size BIGINT,
                mimetype TEXT,
                indexed TIMESTAMP DEFAULT CURRENT_TIMESTAMP)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Tags (
                id SERIAL PRIMARY KEY,
                namespace TEXT,
                descriptor TEXT NOT NULL,
                UNIQUE(namespace, descriptor))"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS TagMap (
                filecid BYTEA NOT NULL,
                tagid INTEGER NOT NULL,
                added TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                reason INTEGER NOT NULL,
                UNIQUE(filecid, tagid),
                FOREIGN KEY (filecid) REFERENCES Files(cid) ON DELETE CASCADE,
                FOREIGN KEY (tagid) REFERENCES Tags(id) ON DELETE CASCADE)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Images (
                cid BYTEA NOT NULL PRIMARY KEY,
                height BIGINT NOT NULL,
                width BIGINT NOT NULL,
                ratio DOUBLE PRECISION NOT NULL,
                primarycolor BYTEA,
                colors BYTEA,
                FOREIGN KEY (cid) REFERENCES Files(cid) ON DELETE CASCADE)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Videos (
                cid BYTEA NOT NULL PRIMARY KEY,
                height BIGINT NOT NULL,
                width BIGINT NOT NULL,
                ratio DOUBLE PRECISION NOT NULL,
                duration DOUBLE PRECISION NOT NULL,
                FOREIGN KEY (cid) REFERENCES Files(cid) ON DELETE CASCADE)"#,
                )
                .await?;

            self.executor
                .execute(
                    r#"
                CREATE TABLE IF NOT EXISTS Thumbnails (
                cid BYTEA NOT NULL PRIMARY KEY,
                size BIGINT,
                mimetype TEXT,
                sourcecid BYTEA NOT NULL,
                height BIGINT NOT NULL,
                width BIGINT NOT NULL,
                ratio DOUBLE PRECISION NOT NULL,
                isanimated BOOLEAN DEFAULT FALSE NOT NULL,
                FOREIGN KEY (sourcecid) REFERENCES Files(cid) ON DELETE CASCADE)"#,
                )
                .await?;

            Ok(())
        }

        // Implement all other methods with PostgreSQL-specific syntax...
        async fn file_tags(&self, cid: Vec<u8>) -> Result<Vec<TagRow>> {
            let tag_rows = sqlx::query::<Postgres>(
                "SELECT id, namespace, descriptor FROM Tags, TagMap WHERE
                filecid = $1 AND tagid = id",
            )
            .bind(cid)
            .try_map(|r: PgRow| {
                let id = r.try_get("id")?;
                let namespace = r.try_get("namespace")?;
                let descriptor = r.try_get("descriptor")?;

                Ok(TagRow {
                    id,
                    namespace,
                    descriptor,
                })
            })
            .fetch_all(&self.executor)
            .await?;

            Ok(tag_rows)
        }

        async fn new_file(&self, f: FileRow) -> Result<()> {
            sqlx::query::<Postgres>(
                r#"
                INSERT INTO files (cid, size, mimetype) VALUES
                ($1, $2, $3) ON CONFLICT DO NOTHING"#,
            )
            .bind(f.cid)
            .bind(f.size)
            .bind(f.mimetype)
            .execute(&self.executor)
            .await?;

            Ok(())
        }

        async fn new_tag_vocab(&self, tags: Vec<Tag>) -> Result<()> {
            if tags.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for t in tags {
                sqlx::query::<Postgres>(
                    r#"
                    INSERT INTO tags (namespace, descriptor) VALUES
                    ($1, $2) ON CONFLICT DO NOTHING"#,
                )
                .bind(t.namespace)
                .bind(t.descriptor)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn new_tag_map(&self, tag_maps: &[TagMapRow]) -> Result<()> {
            if tag_maps.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for t in tag_maps {
                sqlx::query::<Postgres>(
                    r#"
                    INSERT INTO tagmap (filecid, tagid, reason) VALUES
                    ($1, $2, $3) ON CONFLICT DO NOTHING"#,
                )
                .bind(t.file_cid.clone())
                .bind(t.tag_id)
                .bind(t.reason as i32)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn remove_tag_map(
            &self,
            file_cid: Vec<u8>,
            tag_ids: &[i32],
        ) -> Result<()> {
            if tag_ids.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for tag_id in tag_ids {
                sqlx::query::<Postgres>(
                    r#"
                    DELETE FROM TagMap WHERE filecid = $1 AND tagid = $2"#,
                )
                .bind(&file_cid)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn new_thumbnail(&self, thumbnail: ThumbnailRow) -> Result<()> {
            sqlx::query::<Postgres>(
                r#"
                INSERT INTO Thumbnails (Cid, Size, Mimetype, SourceCid, Height, Width, Ratio, IsAnimated) VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING"#,
            )
            .bind(thumbnail.cid)
            .bind(thumbnail.size)
            .bind(thumbnail.mimetype)
            .bind(thumbnail.source_cid)
            .bind(thumbnail.height)
            .bind(thumbnail.width)
            .bind(thumbnail.ratio)
            .bind(thumbnail.is_animated)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn delete_old_thumbnails(&self, cid: Vec<u8>) -> Result<()> {
            sqlx::query::<Postgres>(
                r#"
                DELETE FROM Thumbnails WHERE SourceCid=$1"#,
            )
            .bind(cid)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn new_image(&self, image: ImageRow) -> Result<()> {
            sqlx::query::<Postgres>(
                r#"
                INSERT INTO images (cid, height, width, ratio, primarycolor, colors) VALUES
                ($1, $2, $3, $4, $5, $6) ON CONFLICT(cid)
                    DO UPDATE SET
                    height=EXCLUDED.height, width=EXCLUDED.width,
                    ratio=EXCLUDED.ratio, primarycolor=EXCLUDED.primarycolor,
                    colors=EXCLUDED.colors"#,
            )
            .bind(image.cid)
            .bind(image.height as i64)
            .bind(image.width as i64)
            .bind(image.ratio)
            .bind(image.primary_color)
            .bind(image.colors)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn new_video(&self, video: VideoRow) -> Result<()> {
            sqlx::query::<Postgres>(
                r#"
                INSERT INTO Videos (Cid, Height, Width, Ratio, Duration) VALUES
                ($1, $2, $3, $4, $5) ON CONFLICT(Cid)
                    DO UPDATE SET
                    Height=EXCLUDED.Height, Width=EXCLUDED.Width,
                    Ratio=EXCLUDED.Ratio, Duration=EXCLUDED.Duration"#,
            )
            .bind(video.cid)
            .bind(video.height as i32)
            .bind(video.width as i32)
            .bind(video.ratio)
            .bind(video.duration)
            .execute(&self.executor)
            .await?;
            Ok(())
        }

        async fn lookup_tag_id(&self, tags: Vec<Tag>) -> Result<Vec<TagRow>> {
            if tags.is_empty() {
                return Ok(vec![]);
            }

            let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
                r#"SELECT id, descriptor, namespace
            FROM Tags WHERE (namespace, descriptor) IN ("#,
            );
            let tags_len = tags.len();
            for (i, t) in tags.into_iter().enumerate() {
                builder.push("(");
                builder.push_bind(t.namespace);
                builder.push(",");
                builder.push_bind(t.descriptor);
                builder.push(")");
                if i < tags_len - 1 {
                    builder.push(", ");
                }
            }
            builder.push(")");
            let query = builder.build();

            let tag_rows = query
                .try_map(|r: PgRow| {
                    let descriptor = r.try_get("descriptor")?;
                    let namespace = r.try_get("namespace")?;
                    let id = r.try_get("id")?;

                    Ok(TagRow {
                        id,
                        descriptor,
                        namespace,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(tag_rows)
        }

        async fn file_row(&self, cid: Vec<u8>) -> Result<FileRow> {
            let file_row = sqlx::query::<Postgres>(
                "SELECT cid, mimetype, size FROM files WHERE cid=$1",
            )
            .bind(cid)
            .try_map(|r: PgRow| {
                let cid = r.try_get("cid")?;
                let mimetype = r.try_get("mimetype")?;
                let size = r.try_get("size")?;

                Ok(FileRow {
                    cid,
                    mimetype,
                    size,
                })
            })
            .fetch_one(&self.executor)
            .await?;

            Ok(file_row)
        }

        async fn image_row(&self, cid: Vec<u8>) -> Result<ImageRow> {
            let row =
                sqlx::query::<Postgres>("SELECT cid, height, width, ratio, primarycolor, colors FROM images WHERE cid=$1")
                    .bind(cid)
                    .try_map(|r: PgRow| {
                        let cid = r.try_get("cid")?;
                        let height = r.try_get::<i64, _>("height")? as u32;
                        let width = r.try_get::<i64, _>("width")? as u32;
                        let ratio = r.try_get("ratio")?;
                        let primary_color = r.try_get("primarycolor")?;
                        let colors = r.try_get("colors")?;

                        Ok(ImageRow {
                            cid,
                            height,
                            width,
                            ratio,
                            primary_color,
                            colors,
                        })
                    })
                    .fetch_one(&self.executor)
                    .await?;

            Ok(row)
        }

        async fn video_row(&self, cid: Vec<u8>) -> Result<VideoRow> {
            let row =
                sqlx::query::<Postgres>("SELECT Cid, Height, Width, Ratio, Duration FROM Videos WHERE Cid=$1")
                    .bind(cid)
                    .try_map(|r: PgRow| {
                        let cid = r.try_get("cid")?;
                        let height = r.try_get::<i32, _>("height")? as u32;
                        let width = r.try_get::<i32, _>("width")? as u32;
                        let ratio = r.try_get("ratio")?;
                        let duration = r.try_get("duration")?;

                        Ok(VideoRow {
                            cid,
                            height,
                            width,
                            ratio,
                            duration,
                        })
                    })
                    .fetch_one(&self.executor)
                    .await?;

            Ok(row)
        }

        async fn thumbnails_by_source_cid(
            &self,
            cid: Vec<u8>,
        ) -> Result<Vec<ThumbnailRow>> {
            let thumbnail_rows = sqlx::query::<Postgres>("SELECT Cid, Size, Mimetype, SourceCid, Height, Width, Ratio, IsAnimated FROM Thumbnails WHERE SourceCid=$1")
                .bind(cid)
                .try_map(|r: PgRow| {
                    let cid = r.try_get("cid")?;
                    let size = r.try_get("size")?;
                    let mimetype = r.try_get("mimetype")?;
                    let source_cid = r.try_get("sourcecid")?;
                    let height = r.try_get::<i64, _>("height")?;
                    let width = r.try_get::<i64, _>("width")?;
                    let ratio = r.try_get("ratio")?;
                    let is_animated = r.try_get("isanimated")?;

                    Ok(ThumbnailRow {
                        cid,
                        size,
                        mimetype,
                        source_cid,
                        height,
                        width,
                        ratio,
                        is_animated,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(thumbnail_rows)
        }

        async fn file_page(
            &self,
            count: u32,
            offset: u32,
            oldest_first: bool,
        ) -> Result<Vec<FileRow>> {
            let query = if oldest_first {
                "SELECT cid, mimetype, size FROM files ORDER BY indexed LIMIT $1 OFFSET $2"
            } else {
                "SELECT cid, mimetype, size FROM files ORDER BY indexed DESC LIMIT $1 OFFSET $2"
            };
            let file_rows = sqlx::query::<Postgres>(query)
                .bind(count as i64)
                .bind(offset as i64)
                .try_map(|r: PgRow| {
                    let cid = r.try_get("cid")?;
                    let mimetype = r.try_get("mimetype")?;
                    let size = r.try_get("size")?;

                    Ok(FileRow {
                        cid,
                        mimetype,
                        size,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(file_rows)
        }

        async fn files_page(
            &self,
            query: Option<crate::proto::SearchQuery>,
            page_size: u32,
            page_number: u32,
            sort_order: i32,
            reverse_order: bool,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<(Vec<File>, u32)> {
            let offset = (page_number.saturating_sub(1)) * page_size;

            let order_clause = match (sort_order, reverse_order) {
                (0, true) => "ORDER BY Indexed",
                (0, false) => "ORDER BY Indexed DESC",
                _ => return Err(anyhow::anyhow!("Invalid sort order")),
            };

            let sql_query;
            let count_sql_query;
            let mut params_bind = Vec::new();
            let prepared_statement = if let Some(q) = query {
                let mut where_clause = String::new();
                let mut param_counter = 1;

                if !q.tag_query.is_empty() {
                    for (i, q) in q.tag_query.iter().enumerate() {
                        if i > 0 {
                            where_clause.push_str(" OR ");
                        }
                        if q.negated {
                            where_clause.push_str(" NOT ")
                        }
                        where_clause.push_str(&format!(
                            "(t.namespace = ${} AND t.descriptor = ${})",
                            param_counter,
                            param_counter + 1
                        ));
                        let namespace = q
                            .namespace
                            .clone()
                            .unwrap_or("general".to_string())
                            .to_ascii_lowercase();
                        params_bind.push(namespace);
                        params_bind.push(q.descriptor.to_ascii_lowercase());
                        param_counter += 2;
                    }
                }

                if !where_clause.is_empty() {
                    where_clause.push_str(" AND ");
                }

                let mut visibility_conditions = Vec::new();

                if visibility_filter.includes_public() {
                    visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
                }

                if visibility_filter.includes_unindexed() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
                }

                if visibility_filter.includes_private() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
                }

                if visibility_conditions.is_empty() {
                    where_clause.push_str("1 = 0"); // no visibility specified
                } else {
                    where_clause.push_str(&format!(
                        "({})",
                        visibility_conditions.join(" OR ")
                    ));
                }

                let distinct_tag_count = q.tag_query.len();
                sql_query = format!(
                    r#"
                    SELECT
                        f.cid,
                        f.Size,
                        f.Mimetype,
                        i.height as "ImageHeight",
                        i.width as "ImageWidth",
                        i.ratio as "ImageRatio",
                        i.colors as "ImageColors",
                        i.PrimaryColor as "ImagePrimaryColor",
                        v.height as "VideoHeight",
                        v.width as "VideoWidth",
                        v.ratio as "VideoRatio",
                        v.duration as "VideoDuration"
                    FROM Files f
                    LEFT JOIN Images i ON f.cid = i.Cid
                    LEFT JOIN Videos v ON f.cid = v.Cid
                    INNER JOIN TagMap tm ON f.cid = tm.filecid
                    INNER JOIN Tags t ON t.id = tm.tagid
                    WHERE {where_clause}
                    GROUP BY f.cid, i.Height, i.Width, i.Ratio, i.Colors, i.PrimaryColor, v.Height, v.Width, v.Ratio, v.Duration
                    HAVING COUNT(DISTINCT t.id) = {distinct_tag_count}
                    {order_clause}
                    LIMIT ${} OFFSET ${}
                "#,
                    param_counter,
                    param_counter + 1
                );

                count_sql_query = format!(
                    r#"
                    SELECT COUNT(DISTINCT f.cid) as total
                    FROM Files f
                    LEFT JOIN Images i ON f.cid = i.Cid
                    LEFT JOIN Videos v ON f.cid = v.Cid
                    INNER JOIN TagMap tm ON f.cid = tm.filecid
                    INNER JOIN Tags t ON t.id = tm.tagid
                    WHERE {where_clause}
                "#
                );

                // Build queries with parameters
                let mut query = sqlx::query::<Postgres>(&sql_query);
                let mut count_query = sqlx::query::<Postgres>(&count_sql_query);

                for param in &params_bind {
                    query = query.bind(param);
                    count_query = count_query.bind(param);
                }

                // Add pagination parameters
                query = query.bind(page_size as i64).bind(offset as i64);

                (query, count_query)
            } else {
                let mut visibility_conditions = Vec::new();

                if visibility_filter.includes_public() {
                    visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
                }

                if visibility_filter.includes_unindexed() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
                }

                if visibility_filter.includes_private() {
                    visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
                }

                let visibility_where = if visibility_conditions.is_empty() {
                    "WHERE 1 = 0" // no visibility types allowed
                } else {
                    &format!("WHERE ({})", visibility_conditions.join(" OR "))
                };

                sql_query = format!(
                    r#"
                    SELECT
                        f.cid,
                        f.size,
                        f.mimetype,
                        i.height as "ImageHeight",
                        i.width as "ImageWidth",
                        i.ratio as "ImageRatio",
                        i.colors as "ImageColors",
                        i.primarycolor as "ImagePrimaryColor",
                        v.height as "VideoHeight",
                        v.width as "VideoWidth",
                        v.ratio as "VideoRatio",
                        v.duration as "VideoDuration"
                    FROM files f
                    LEFT JOIN images i ON f.cid = i.cid
                    LEFT JOIN videos v ON f.cid = v.cid
                    {visibility_where}
                    {order_clause}
                    LIMIT $1 OFFSET $2
                "#
                );
                count_sql_query = format!(
                    r#"SELECT COUNT(*) as total FROM files f {visibility_where}"#
                );

                let query = sqlx::query::<Postgres>(&sql_query)
                    .bind(page_size as i64)
                    .bind(offset as i64);
                let count_query = sqlx::query::<Postgres>(&count_sql_query);
                (query, count_query)
            };

            let raw_files =
                prepared_statement.0.fetch_all(&self.executor).await?;

            let total_count: i64 = prepared_statement
                .1
                .fetch_one(&self.executor)
                .await?
                .try_get("total")?;
            let total_count = total_count as u32;
            let final_page_token = if total_count % page_size == 0 {
                total_count / page_size
            } else {
                (total_count / page_size) + 1
            };

            let mut files: Vec<File> = Vec::with_capacity(raw_files.len());
            for raw_file in raw_files.into_iter() {
                let cid: Vec<u8> = raw_file.try_get("cid").map_err(|e| {
                    tracing::error!("files_page failed to read cid: {:?}", e);
                    e
                })?;
                let thumbnails =
                    crate::local::fetch_thumbnails_for(self, &cid).await?;

                // fetch tags for this file using existing method
                let tag_rows = self.file_tags(cid.clone()).await?;
                let tags = tag_rows
                    .into_iter()
                    .map(|tag_row| Tag {
                        namespace: tag_row.namespace,
                        descriptor: tag_row.descriptor,
                    })
                    .collect();

                let ext_file = if let Ok(height) =
                    raw_file.try_get::<Option<i64>, _>("ImageHeight").map_err(|e| {
                        tracing::error!("files_page failed to read ImageHeight for CID {}: {:?}", 
                            crate::cid::encode(&cid), e);
                        e
                    })
                {
                    if let Some(height) = height {
                        let colors_data: Option<Vec<u8>> =
                            raw_file.try_get("ImageColors")?;
                        let colors = colors_data
                            .unwrap_or_default()
                            .chunks(3)
                            .map(|c| c.into())
                            .collect();
                        Some(crate::proto::file::ExtFile::Image(
                            crate::proto::Image {
                                height,
                                width: raw_file
                                    .try_get::<Option<i64>, _>("ImageWidth")?
                                    .unwrap_or(0),
                                aspect_ratio: raw_file
                                    .try_get::<Option<f64>, _>("ImageRatio")?
                                    .unwrap_or(0.0)
                                    as f32,
                                colors,
                                thumbnails,
                            },
                        ))
                    } else {
                        None
                    }
                } else if let Ok(height) =
                    raw_file.try_get::<Option<i64>, _>("VideoHeight").map_err(|e| {
                        tracing::error!("files_page failed to read VideoHeight for CID {}: {:?}", 
                            crate::cid::encode(&cid), e);
                        e
                    })
                {
                    if let Some(height) = height {
                        Some(crate::proto::file::ExtFile::Video(
                            crate::proto::Video {
                                height,
                                width: raw_file
                                    .try_get::<Option<i64>, _>("VideoWidth")?
                                    .unwrap_or(0),
                                aspect_ratio: raw_file
                                    .try_get::<Option<f64>, _>("VideoRatio")?
                                    .unwrap_or(0.0)
                                    as f32,
                                thumbnails,
                                duration: raw_file
                                    .try_get::<Option<f64>, _>("VideoDuration")?
                                    .unwrap_or(0.0)
                                    as f32,
                            },
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };

                files.push(File {
                    cid,
                    size: raw_file.try_get("size")?,
                    mimetype: raw_file.try_get("mimetype")?,
                    processing_status: Default::default(),
                    ext_file,
                    tags,
                });
            }

            Ok((files, final_page_token))
        }

        async fn tags_page(
            &self,
            page_size: u32,
            page_number: u32,
            sort_order: i32,
            reverse_order: bool,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<(Vec<TagRowCount>, u32)> {
            let offset = (page_number.saturating_sub(1)) * page_size;

            let order_clause = match (sort_order, reverse_order) {
                (0, true) => "ORDER BY Count",
                (0, false) => "ORDER BY Count DESC",
                _ => return Err(anyhow::anyhow!("Invalid sort order")),
            };

            let total_count_query = r#"
                SELECT COUNT(DISTINCT Id) as total_count
                FROM Tags
            "#;

            let total_count_row = sqlx::query::<Postgres>(total_count_query)
                .fetch_one(&self.executor)
                .await?;

            let total_count: i64 = total_count_row.try_get("total_count")?;
            let total_count = total_count as u32;
            let final_page_token = if page_size > 0 {
                total_count.div_ceil(page_size)
            } else {
                1
            };

            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            let visibility_where = if visibility_conditions.is_empty() {
                "WHERE 1 = 0" // no visibility types allowed
            } else {
                &format!("WHERE ({})", visibility_conditions.join(" OR "))
            };

            let query = format!(
                r#"
                    SELECT
                        t.id,
                        t.namespace,
                        t.descriptor,
                        COUNT(*) as "count"
                    FROM Tags t
                    INNER JOIN TagMap tm ON t.id = tm.tagid
                    INNER JOIN Files f ON f.cid = tm.filecid
                    {visibility_where}
                    GROUP BY t.id, t.namespace, t.descriptor
                    {order_clause}
                    LIMIT $1 OFFSET $2
                "#
            );

            let prepared_statement = sqlx::query::<Postgres>(&query)
                .bind(page_size as i64)
                .bind(offset as i64)
                .try_map(|t: PgRow| {
                    Ok(TagRowCount {
                        id: t.try_get("id")?,
                        namespace: t.try_get("namespace")?,
                        descriptor: t.try_get("descriptor")?,
                        count: t.try_get("count")?,
                    })
                });

            let tags_info =
                prepared_statement.fetch_all(&self.executor).await?;

            Ok((tags_info, final_page_token))
        }

        async fn get_most_popular_tags(
            &self,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<Vec<TagRowCount>> {
            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            let visibility_where = if visibility_conditions.is_empty() {
                "WHERE 1 = 0" // no visibility types allowed
            } else {
                &format!("WHERE ({})", visibility_conditions.join(" OR "))
            };

            let query = format!(
                r#"
                SELECT t.id, t.namespace, t.descriptor, COUNT(*) AS associations FROM Tags t
                INNER JOIN TagMap tm ON t.id = tm.tagid
                INNER JOIN Files f ON tm.filecid = f.cid
                {visibility_where}
                GROUP BY t.id, t.namespace, t.descriptor
                ORDER BY associations DESC
                LIMIT 10
                "#
            );

            let ret = sqlx::query::<Postgres>(&query)
                .try_map(|r: PgRow| {
                    let count = r.try_get("associations")?;
                    let descriptor = r.try_get("descriptor")?;
                    let namespace = r.try_get("namespace")?;
                    let id = r.try_get("id")?;
                    Ok(TagRowCount {
                        id,
                        namespace,
                        descriptor,
                        count,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(ret)
        }

        async fn get_most_popular_tags_within_namespace_that_starts_with(
            &self,
            tag_constraints: &[crate::proto::TagQuery],
            begins_with: &str,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<Vec<TagRowCount>> {
            let mut params_bind = Vec::new();
            let mut param_counter = 1;

            let mut where_clauses = vec![];

            let like_clause = format!("{begins_with}%");
            params_bind.push(like_clause);
            where_clauses.push(format!("t.namespace LIKE ${param_counter}"));
            param_counter += 1;

            for constraint in tag_constraints {
                let negation_clause =
                    if constraint.negated { "NOT" } else { "" };

                let namespace = constraint
                    .namespace
                    .clone()
                    .unwrap_or_else(|| "general".to_string());

                params_bind.push(namespace);
                params_bind.push(constraint.descriptor.clone());

                let subquery = format!(
                    r#"
                    {negation_clause} EXISTS (
                        SELECT 1 FROM TagMap tm
                        INNER JOIN Tags tg ON tm.tagid = tg.Id
                        WHERE tm.filecid = f.cid AND tg.namespace = ${} AND tg.descriptor = ${}
                    )"#,
                    param_counter,
                    param_counter + 1
                );
                where_clauses.push(subquery);
                param_counter += 2;
            }

            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            if !visibility_conditions.is_empty() {
                where_clauses
                    .push(format!("({})", visibility_conditions.join(" OR ")));
            } else {
                where_clauses.push("1 = 0".to_string()); // no visibility types allowed
            }

            let having_clause = if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                format!("HAVING COUNT(DISTINCT tm.filecid) >= ${param_counter}")
            } else {
                String::new()
            };

            let sql_query = format!(
                r#"
                SELECT t.id, t.namespace, t.descriptor, COUNT(DISTINCT tm.filecid) AS associations FROM Tags t
                INNER JOIN TagMap tm ON t.id = tm.tagid
                INNER JOIN Files f ON tm.filecid = f.cid
                WHERE {}
                GROUP BY t.id, t.namespace, t.descriptor
                {}
                LIMIT 10
            "#,
                where_clauses.join(" AND "),
                having_clause
            );

            let mut prepared_statement = sqlx::query::<Postgres>(&sql_query);
            for param in params_bind {
                prepared_statement = prepared_statement.bind(param);
            }

            if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                prepared_statement =
                    prepared_statement.bind(non_negated_count as i64);
            }

            let res = prepared_statement
                .try_map(|r: PgRow| {
                    let count = r.try_get("associations")?;
                    let descriptor = r.try_get("descriptor")?;
                    let namespace = r.try_get("namespace")?;
                    let id = r.try_get("id")?;
                    Ok(TagRowCount {
                        id,
                        count,
                        namespace,
                        descriptor,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(res)
        }

        async fn get_descriptors_that_start_with(
            &self,
            tag_constraints: &[crate::proto::TagQuery],
            namespace: Option<String>,
            begins_with: &str,
            visibility_filter: crate::visibility::VisibilityFilter,
        ) -> Result<Vec<TagRowCount>> {
            let mut params_bind = Vec::new();
            let mut param_counter = 1;
            let mut where_clauses = vec![];

            let like_clause = format!("{begins_with}%");
            params_bind.push(like_clause);
            where_clauses.push(format!("t.descriptor LIKE ${param_counter}"));
            param_counter += 1;

            if let Some(n) = namespace {
                params_bind.push(n);
                where_clauses.push(format!("t.namespace = ${param_counter}"));
                param_counter += 1;
            }

            for constraint in tag_constraints {
                let negation_clause =
                    if constraint.negated { "NOT" } else { "" };

                let namespace = constraint
                    .namespace
                    .clone()
                    .unwrap_or_else(|| "general".to_string());

                params_bind.push(namespace);
                params_bind.push(constraint.descriptor.clone());

                let subquery = format!(
                    r#"
                    {negation_clause} EXISTS (
                        SELECT 1 FROM TagMap tm
                        INNER JOIN Tags tg ON tm.tagid = tg.Id
                        WHERE tm.filecid = f.cid AND tg.namespace = ${} AND tg.descriptor = ${}
                    )"#,
                    param_counter,
                    param_counter + 1
                );
                where_clauses.push(subquery);
                param_counter += 2;
            }

            let mut visibility_conditions = Vec::new();

            if visibility_filter.includes_public() {
                visibility_conditions.push("NOT EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility')".to_string());
            }

            if visibility_filter.includes_unindexed() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'unindexed')".to_string());
            }

            if visibility_filter.includes_private() {
                visibility_conditions.push("EXISTS (SELECT 1 FROM TagMap tm_vis INNER JOIN Tags t_vis ON tm_vis.tagid = t_vis.id WHERE tm_vis.filecid = f.cid AND t_vis.namespace = 'visibility' AND t_vis.descriptor = 'private')".to_string());
            }

            if !visibility_conditions.is_empty() {
                where_clauses
                    .push(format!("({})", visibility_conditions.join(" OR ")));
            } else {
                where_clauses.push("1 = 0".to_string()); // no visibility types allowed
            }

            let having_clause = if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                format!("HAVING COUNT(DISTINCT tm.filecid) >= ${param_counter}")
            } else {
                String::new()
            };

            let sql_query = format!(
                r#"
                SELECT t.id, t.namespace, t.descriptor, COUNT(DISTINCT tm.filecid) AS associations FROM Tags t
                INNER JOIN TagMap tm ON t.id = tm.tagid
                INNER JOIN Files f ON tm.filecid = f.cid
                WHERE {}
                GROUP BY t.id, t.namespace, t.descriptor
                {}
                LIMIT 10
            "#,
                where_clauses.join(" AND "),
                having_clause
            );

            let mut prepared_statement = sqlx::query::<Postgres>(&sql_query);
            for param in params_bind {
                prepared_statement = prepared_statement.bind(param);
            }

            if !tag_constraints.is_empty() {
                let non_negated_count =
                    tag_constraints.iter().filter(|tc| !tc.negated).count();
                prepared_statement =
                    prepared_statement.bind(non_negated_count as i64);
            }

            let res = prepared_statement
                .try_map(|r: PgRow| {
                    let count = r.try_get("associations")?;
                    let descriptor = r.try_get("descriptor")?;
                    let namespace = r.try_get("namespace")?;
                    let id = r.try_get("id")?;
                    Ok(TagRowCount {
                        id,
                        count,
                        namespace,
                        descriptor,
                    })
                })
                .fetch_all(&self.executor)
                .await?;

            Ok(res)
        }

        async fn random_file(&self, count: u32) -> Result<Vec<FileRow>> {
            let file_rows = sqlx::query::<Postgres>(
                "SELECT cid, mimetype, size FROM files ORDER BY RANDOM() LIMIT $1",
            )
            .bind(count as i64)
            .try_map(|r: PgRow| {
                let cid = r.try_get("cid")?;
                let mimetype = r.try_get("mimetype")?;
                let size = r.try_get("size")?;

                Ok(FileRow {
                    cid,
                    mimetype,
                    size,
                })
            })
            .fetch_all(&self.executor)
            .await?;

            Ok(file_rows)
        }

        async fn count_files(&self) -> Result<i64> {
            let row =
                sqlx::query::<Postgres>("SELECT COUNT(*) as count FROM files")
                    .fetch_one(&self.executor)
                    .await?;
            Ok(row.get("count"))
        }

        async fn count_tags(&self) -> Result<i64> {
            let row =
                sqlx::query::<Postgres>("SELECT COUNT(*) as count FROM Tags")
                    .fetch_one(&self.executor)
                    .await?;
            Ok(row.get("count"))
        }

        async fn count_tag_associations(&self) -> Result<i64> {
            let row =
                sqlx::query::<Postgres>("SELECT COUNT(*) as count FROM TagMap")
                    .fetch_one(&self.executor)
                    .await?;
            Ok(row.get("count"))
        }

        async fn delete_file(&self, cid: Vec<u8>) -> Result<()> {
            sqlx::query::<Postgres>("DELETE FROM files WHERE cid = $1")
                .bind(cid)
                .execute(&self.executor)
                .await?;
            Ok(())
        }

        async fn batch_new_tag_maps(
            &self,
            all_tag_maps: &[TagMapRow],
        ) -> Result<()> {
            if all_tag_maps.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for t in all_tag_maps {
                sqlx::query::<Postgres>(
                    r#"
                    INSERT INTO TagMap (filecid, tagid, reason) VALUES
                    ($1, $2, $3) ON CONFLICT DO NOTHING"#,
                )
                .bind(t.file_cid.clone())
                .bind(t.tag_id)
                .bind(t.reason as i32)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }

        async fn batch_remove_tag_maps(
            &self,
            removals: &[(Vec<u8>, i32)],
        ) -> Result<()> {
            if removals.is_empty() {
                return Ok(());
            }

            let mut tx = self.executor.begin().await?;

            for (file_cid, tag_id) in removals {
                sqlx::query::<Postgres>(
                    r#"
                    DELETE FROM TagMap WHERE filecid = $1 AND tagid = $2"#,
                )
                .bind(file_cid)
                .bind(tag_id)
                .execute(&mut *tx)
                .await?;
            }

            tx.commit().await?;
            Ok(())
        }
    }
}

// Re-export the backend implementations
pub use postgres::PostgresBackend;
pub use sqlite::SqliteBackend;
