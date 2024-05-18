use anyhow::Result;
use sqlx::{
    sqlite::SqliteRow, Executor, QueryBuilder, Row, Sqlite, SqlitePool,
};

use crate::proto::{file::ExtFile, File, Tag};

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

pub struct Db {
    executor: SqlitePool,
}

impl Db {
    pub fn new(executor: SqlitePool) -> Self {
        Self { executor }
    }

    pub async fn init_tables(&mut self) -> Result<()> {
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

    pub async fn file_tags(&self, cid: Vec<u8>) -> Result<Vec<TagRow>> {
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

    pub async fn new_file(&self, f: FileRow) -> sqlx::Result<()> {
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

    pub async fn new_tag_vocab(&self, tags: Vec<Tag>) -> Result<()> {
        for t in tags {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO Tags (Namespace, Descriptor) VALUES
                (?, ?)"#,
            )
            .bind(t.namespace)
            .bind(t.descriptor)
            .execute(&self.executor)
            .await?;
        }

        Ok(())
    }

    pub async fn new_tag_map(&self, tag_maps: &[TagMapRow]) -> Result<()> {
        for t in tag_maps {
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO TagMap (FileCid, TagId, Added, Reason) VALUES
                (?, ?, ?, ?)"#,
            )
            .bind(t.file_cid.clone())
            .bind(t.tag_id)
            .bind(t.added.clone())
            .bind(t.reason)
            .execute(&self.executor)
            .await?;
        }

        Ok(())
    }

    pub async fn new_thumbnail(&self, thumbnail: ThumbnailRow) -> Result<()> {
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

    pub async fn delete_old_thumbnails(&self, cid: Vec<u8>) -> Result<()> {
        sqlx::query(
            r#"
            DELETE FROM Thumbnails WHERE SourceCid=?"#,
        )
        .bind(cid)
        .execute(&self.executor)
        .await?;
        Ok(())
    }

    pub async fn new_image(&self, image: ImageRow) -> Result<()> {
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
        .bind(image.height)
        .bind(image.width)
        .bind(image.ratio)
        .bind(image.primary_color)
        .bind(image.colors)
        .execute(&self.executor)
        .await?;
        Ok(())
    }

    pub async fn new_video(&self, image: VideoRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO Videos (Cid, Height, Width, Ratio, Duration) VALUES
            (?, ?, ?, ?, ?) ON CONFLICT(Cid)
                DO UPDATE SET
                Height=excluded.Height, Width=excluded.Width,
                Ratio=excluded.Ratio, Duration=excluded.Duration"#,
        )
        .bind(image.cid)
        .bind(image.height)
        .bind(image.width)
        .bind(image.ratio)
        .bind(image.duration)
        .execute(&self.executor)
        .await?;
        Ok(())
    }

    pub async fn lookup_tag_id(&self, tags: Vec<Tag>) -> Result<Vec<TagRow>> {
        if tags.is_empty() {
            return Ok(vec![]);
        }

        let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new(
            r#"SELECT Id, Descriptor, Namespace
        FROM Tags WHERE (Namespace, Descriptor) IN ("#,
        );
        let tags_len = tags.len();
        for (i, t) in tags.into_iter().enumerate() {
            if i < tags_len - 1 {
                builder.push(", ");
            }
            builder.push("(");
            builder.push_bind(t.namespace);
            builder.push(",");
            builder.push_bind(t.descriptor);
            builder.push(")");
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

    pub async fn file_row(&self, cid: Vec<u8>) -> Result<FileRow> {
        let file_row =
            sqlx::query("SELECT Cid, Mimetype, Size FROM Files WHERE Cid=?")
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

    pub async fn image_row(&self, cid: Vec<u8>) -> Result<ImageRow> {
        let row =
            sqlx::query("SELECT Cid, Height, Width, Ratio, PrimaryColor, Colors FROM Images WHERE Cid=?")
                .bind(cid)
                .try_map(|r: SqliteRow| {
                    let cid = r.try_get("Cid")?;
                    let height = r.try_get("Height")?;
                    let width = r.try_get("Width")?;
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

    pub async fn video_row(&self, cid: Vec<u8>) -> Result<VideoRow> {
        let row =
            sqlx::query("SELECT Cid, Height, Width, Ratio, Duration FROM Videos WHERE Cid=?")
                .bind(cid)
                .try_map(|r: SqliteRow| {
                    let cid = r.try_get("Cid")?;
                    let height = r.try_get("Height")?;
                    let width = r.try_get("Width")?;
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

    pub async fn thumbnails_by_source_cid(
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
                let height = r.try_get("Height")?;
                let width = r.try_get("Width")?;
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

    pub async fn file_page(
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
    pub async fn files_page(
        &self,
        query: Option<crate::proto::SearchQuery>,
        page_size: u32,
        page_number: u32,
        sort_order: i32,
        reverse_order: bool,
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
                        .push_str("(t.Namespace = ? AND t.Descriptor = ?)");
                }
            }

            let distinct_tag_count = q.tag_query.len();
            sql_query = format!(
                r#"
                SELECT
                    f.Cid,
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
                LEFT JOIN Images i ON f.Cid = i.Cid
                LEFT JOIN Videos v ON f.Cid = v.Cid
                INNER JOIN TagMap tm ON f.Cid = tm.FileCid
                INNER JOIN Tags t ON t.Id = tm.TagId
                WHERE {}
                GROUP BY f.Cid
                HAVING COUNT(DISTINCT t.Id) = {}
                {}
                LIMIT ? OFFSET ?
            "#,
                where_clause, distinct_tag_count, order_clause
            );

            count_sql_query = format!(
                r#"
                SELECT COUNT(DISTINCT f.Cid) as total
                FROM Files f
                LEFT JOIN Images i ON f.Cid = i.Cid
                LEFT JOIN Videos v ON f.Cid = v.Cid
                INNER JOIN TagMap tm ON f.Cid = tm.FileCid
                INNER JOIN Tags t ON t.Id = tm.TagId
                WHERE {}
            "#,
                where_clause
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

                query = query.bind(namespace.clone()).bind(descriptor.clone());
                count_query = count_query.bind(namespace).bind(descriptor);
            }

            // Lastly bind the pagination parameters
            query = query.bind(page_size).bind(offset);

            (query, count_query)
        } else {
            sql_query = format!(
                r#"
                SELECT
                    f.Cid,
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
                LEFT JOIN Images i ON f.Cid = i.Cid
                LEFT JOIN Videos v ON f.Cid = v.Cid
                {}
                LIMIT ? OFFSET ?
            "#,
                order_clause
            );
            count_sql_query =
                r#"SELECT COUNT(*) as total FROM Files f"#.to_string();

            let query = sqlx::query(&sql_query).bind(page_size).bind(offset);
            let count_query = sqlx::query(&count_sql_query);
            (query, count_query)
        };

        let raw_files = prepared_statement.0.fetch_all(&self.executor).await?;

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
            let thumbnails =
                self.fetch_thumbnails_for(raw_file.try_get("Cid")?).await?;
            let ext_file = if let Ok(height) = raw_file.try_get("ImageHeight?")
            {
                let colors = raw_file
                    .try_get::<Vec<u8>, _>("ImageColors?")?
                    .chunks(3)
                    .map(|c| c.into())
                    .collect();
                Some(ExtFile::Image(crate::proto::Image {
                    height,
                    width: raw_file.try_get("ImageWidth?")?,
                    aspect_ratio: raw_file.try_get("ImageRatio?")?,
                    colors,
                    thumbnails,
                }))
            } else if let Ok(height) = raw_file.try_get("VideoHeight?") {
                Some(ExtFile::Video(crate::proto::Video {
                    height,
                    width: raw_file.try_get("VideoWidth")?,
                    aspect_ratio: raw_file.try_get("VideoRatio?")?,
                    thumbnails,
                    duration: raw_file.try_get("VideoDuration?")?,
                }))
            } else {
                None
            };

            files.push(File {
                cid: raw_file.try_get("Cid")?,
                size: raw_file.try_get("Size")?,
                mimetype: raw_file.try_get("Mimetype")?,
                ext_file,
            });
        }

        Ok((files, final_page_token))
    }

    pub async fn tags_page(
        &self,
        page_size: u32,
        page_number: u32,
        sort_order: i32,
        reverse_order: bool,
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
            (total_count + page_size - 1) / page_size
        } else {
            1
        };

        let query = format!(
            r#"
                SELECT
                    t.Id,
                    t.Namespace,
                    t.Descriptor,
                    COUNT(*) as "Count"
                FROM Tags t
                INNER JOIN TagMap tm ON t.Id = tm.TagId
                INNER JOIN Files f ON f.Cid = tm.FileCid
                GROUP BY t.Id
                {}
                LIMIT ? OFFSET ?
            "#,
            order_clause
        );

        let prepared_statement = sqlx::query(&query)
            .bind(page_size)
            .bind(offset)
            .try_map(|t: SqliteRow| {
                Ok(TagRowCount {
                    id: t.try_get("Id")?,
                    namespace: t.try_get("Namespace")?,
                    descriptor: t.try_get("Descriptor")?,
                    count: t.try_get("Count")?,
                })
            });

        let tags_info = prepared_statement.fetch_all(&self.executor).await?;

        Ok((tags_info, final_page_token))
    }

    async fn fetch_thumbnails_for(
        &self,
        source_cid: &[u8],
    ) -> Result<Vec<crate::proto::Thumbnail>> {
        let thumbnails = sqlx::query(
            r#"
            SELECT
                Cid,
                Size,
                Mimetype,
                SourceCid,
                Height,
                Width,
                Ratio,
                IsAnimated
            FROM Thumbnails
            WHERE SourceCid = ?
            "#,
        )
        .bind(source_cid)
        .try_map(|t: SqliteRow| {
            Ok(crate::proto::Thumbnail {
                cid: t.try_get("Cid")?,
                size: t.try_get("Size")?,
                mimetype: t.try_get("Mimetype")?,
                source_cid: t.try_get("SourceCid")?,
                height: t.try_get("Height")?,
                width: t.try_get("Width")?,
                aspect_ratio: t.try_get("Ratio")?,
                is_animated: t.try_get("IsAnimated")?,
            })
        })
        .fetch_all(&self.executor)
        .await?
        .into_iter()
        .collect();

        Ok(thumbnails)
    }

    pub async fn get_most_popular_tags(&self) -> Result<Vec<TagRowCount>> {
        let ret = sqlx::query(r#"
            SELECT Id, Namespace, Descriptor, COUNT(*) AS Associations FROM Tags t
            INNER JOIN TagMap tm ON t.Id = tm.TagId
            GROUP BY t.Id
            ORDER BY Associations DESC
            LIMIT 10
            "#)
        .try_map(|r: SqliteRow| {
            let count = r.try_get("Associations")?;
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

    pub async fn get_most_popular_tags_within_namespace_that_starts_with(
        &self,
        tag_constraints: &[crate::proto::TagQuery],
        begins_with: &str,
    ) -> Result<Vec<TagRowCount>> {
        let mut params_bind = Vec::new();

        let mut where_clauses = vec![];

        let like_clause = format!("{}%", begins_with);
        params_bind.push(like_clause);
        where_clauses.push("t.Namespace LIKE ?".to_string());

        for constraint in tag_constraints {
            let negation_clause = if constraint.negated { "NOT" } else { "" };

            let namespace = constraint
                .namespace
                .clone()
                .unwrap_or_else(|| "general".to_string());

            params_bind.push(namespace);
            params_bind.push(constraint.descriptor.clone());

            let subquery = format!(
                r#"
                {} EXISTS (
                    SELECT 1 FROM TagMap tm
                    INNER JOIN Tags tg ON tm.TagId = tg.Id
                    WHERE tm.FileCid = f.Cid AND tg.Namespace = ? AND tg.Descriptor = ?
                )"#,
                negation_clause,
            );
            where_clauses.push(subquery);
        }

        let having_clause = if !tag_constraints.is_empty() {
            let non_negated_count =
                tag_constraints.iter().filter(|tc| !tc.negated).count();
            format!(
                "HAVING COUNT(DISTINCT tm.FileCid) >= {}",
                non_negated_count
            )
        } else {
            String::new()
        };

        let sql_query = format!(
            r#"
            SELECT t.Id, t.Namespace, t.Descriptor, COUNT(DISTINCT tm.FileCid) AS Associations FROM Tags t
            INNER JOIN TagMap tm ON t.Id = tm.TagId
            INNER JOIN Files f ON tm.FileCid = f.Cid
            WHERE {}
            GROUP BY t.Id
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
                let count = r.try_get("Associations")?;
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

    pub async fn get_descriptors_that_start_with(
        &self,
        tag_constraints: &[crate::proto::TagQuery],
        namespace: Option<String>,
        begins_with: &str,
    ) -> Result<Vec<TagRowCount>> {
        let mut params_bind = Vec::new();

        let mut where_clauses = vec![];

        let like_clause = format!("{}%", begins_with);
        params_bind.push(like_clause);
        where_clauses.push("t.Descriptor LIKE ?".to_string());

        if let Some(n) = namespace {
            params_bind.push(n);
            where_clauses.push("t.Namespace = ?".to_string());
        }

        for constraint in tag_constraints {
            let negation_clause = if constraint.negated { "NOT" } else { "" };

            let namespace = constraint
                .namespace
                .clone()
                .unwrap_or_else(|| "general".to_string());

            params_bind.push(namespace);
            params_bind.push(constraint.descriptor.clone());

            let subquery = format!(
                r#"
                {} EXISTS (
                    SELECT 1 FROM TagMap tm
                    INNER JOIN Tags tg ON tm.TagId = tg.Id
                    WHERE tm.FileCid = f.Cid AND tg.Namespace = ? AND tg.Descriptor = ?
                )"#,
                negation_clause,
            );
            where_clauses.push(subquery);
        }

        let having_clause = if !tag_constraints.is_empty() {
            let non_negated_count =
                tag_constraints.iter().filter(|tc| !tc.negated).count();
            format!(
                "HAVING COUNT(DISTINCT tm.FileCid) >= {}",
                non_negated_count
            )
        } else {
            String::new()
        };

        let sql_query = format!(
            r#"
            SELECT t.Id, t.Namespace, t.Descriptor, COUNT(DISTINCT tm.FileCid) AS Associations FROM Tags t
            INNER JOIN TagMap tm ON t.Id = tm.TagId
            INNER JOIN Files f ON tm.FileCid = f.Cid
            WHERE {}
            GROUP BY t.Id
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
                let count = r.try_get("Associations")?;
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

    pub async fn random_file(&self, count: u32) -> Result<Vec<FileRow>> {
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
}
