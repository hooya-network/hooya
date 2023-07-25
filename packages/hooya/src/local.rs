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

        Ok(())
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
