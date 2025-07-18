use anyhow::Result;
use chrono::NaiveDate;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufReader, SeekFrom};
use tokio::sync::RwLock;

pub struct Chatroom {
    filestore_path: PathBuf,
    day_cache: RwLock<HashMap<String, NaiveDate>>,
}

impl Chatroom {
    pub fn new(filestore_path: PathBuf) -> Self {
        Self {
            filestore_path,
            day_cache: RwLock::new(HashMap::new()),
        }
    }

    pub async fn log_message(
        &self,
        channel: &str,
        sender: &str,
        content: &str,
    ) -> Result<()> {
        let today = chrono::Local::now().date_naive();
        let mut day_cache = self.day_cache.write().await;

        // ensure chat_history directory exists
        let history_path = self.filestore_path.join("chat_history");
        tokio::fs::create_dir_all(&history_path).await?;

        let log_path = crate::runtime::sanitize_filestore_path(
            &self.filestore_path,
            "chat_history",
            &format!("{channel}.log"),
        )?;

        // check if this is a new file that needs a date header
        let is_new_file = !log_path.exists();

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .await?;

        // if it's a new file, add the initial date header
        if is_new_file {
            let separator =
                format!("--- Day changed {}\n", today.format("%a %b %d %Y"));
            file.write_all(separator.as_bytes()).await?;
            day_cache.insert(channel.to_string(), today);
        }

        // check if day changed - consult earlier lines if not cached
        let last_date = match day_cache.get(channel) {
            Some(date) => *date,
            None => {
                let date = self
                    .get_last_date_from_log(&log_path)
                    .await
                    .unwrap_or(today);
                day_cache.insert(channel.to_string(), date);
                date
            }
        };

        // insert day separator if day changed
        if today != last_date {
            let separator =
                format!("--- Day changed {}\n", today.format("%a %b %d %Y"));
            file.write_all(separator.as_bytes()).await?;
        }

        // update cache with current date
        day_cache.insert(channel.to_string(), today);

        // write the actual message
        let timestamp = chrono::Local::now().format("%H:%M");
        let line = format!("{timestamp}  <{sender}> {content}\n");
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;

        Ok(())
    }

    async fn get_last_date_from_log(
        &self,
        log_path: &Path,
    ) -> Option<NaiveDate> {
        let file = tokio::fs::File::open(log_path).await.ok()?;
        let reader = BufReader::new(file);
        let mut _lines: Vec<String> = Vec::new();

        // read backwards through the file to find the last "Day changed" line
        let mut file_handle = reader.into_inner();
        let file_size = file_handle.metadata().await.ok()?.len();

        if file_size == 0 {
            return None;
        }

        // read the last chunk of the file (last 4KB should be enough for recent messages)
        let read_size = std::cmp::min(4096, file_size);
        let start_pos = file_size - read_size;

        file_handle.seek(SeekFrom::Start(start_pos)).await.ok()?;
        let mut buffer = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut file_handle, &mut buffer)
            .await
            .ok()?;

        let content = String::from_utf8_lossy(&buffer);

        // find the last "--- Day changed" line
        for line in content.lines().rev() {
            if line.starts_with("--- Day changed ") {
                // parse the date from "--- Day changed Sun Dec 31 2017"
                let date_str = line.strip_prefix("--- Day changed ")?;
                return chrono::NaiveDate::parse_from_str(
                    date_str,
                    "%a %b %d %Y",
                )
                .ok();
            }
        }

        None
    }
}
