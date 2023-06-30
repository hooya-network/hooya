use sqlx::{Executor, SqlitePool};

pub struct TagRow {}

pub struct FileRow {
    pub cid: Vec<u8>,
    pub size: i64,
    pub mimetype: String,
}

pub struct TagMapRow {}

pub struct Db {
    executor: SqlitePool,
}

impl Db {
    pub fn new(executor: SqlitePool) -> Self {
        Self { executor }
    }

    pub async fn init_tables(&mut self) -> sqlx::Result<()> {
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
            FileCid TEXT NOT NULL,
            TagId TEXT NOT NULL,
            ADDED DATETIME DEFAULT CURRENT_TIMESTAMP,
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
}
