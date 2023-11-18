use anyhow::Result;
use sqlx::{
    sqlite::SqliteRow, Executor, QueryBuilder, Row, Sqlite, SqlitePool,
};

use crate::proto::Tag;

pub struct TagRow {
    pub id: i32,
    pub namespace: String,
    pub descriptor: String,
}

pub struct FileRow {
    pub cid: Vec<u8>,
    pub size: i64,
    pub mimetype: Option<String>,
}

#[derive(Debug)]
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

    pub async fn new_image(&self, image: ImageRow) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO Images (Cid, Height, Width, Ratio, PrimaryColor, Colors) VALUES
            (?, ?, ?, ?, ?, ?)"#,
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
