use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::time::Instant;
use tracing::{debug, info};

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub password: Option<String>,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendChatMessageRequest {
    pub channel: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendChatMessageResponse {
    pub message_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemInfo {
    pub instance_name: String,
    pub operator_name: String,
    pub node_id: String,
    pub stats: SystemStats,
    pub daemon_version: VersionInfo,
    pub webui_version: VersionInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemStats {
    pub files_indexed: i64,
    pub associations_count: i64,
    pub tags_count: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionInfo {
    pub version_string: String,
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTag {
    pub namespace: String,
    pub descriptor: String,
}

#[derive(Debug, Deserialize)]
pub struct StartUploadResponse {
    pub upload_id: String,
    pub chunk_size: u32,
}

#[derive(Debug, Deserialize)]
pub struct UploadChunkResponse {
    pub status: i32,
    pub bytes_received: u64,
    pub next_chunk_index: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CompleteUploadResponse {
    pub cid: String,
    pub file: Option<serde_json::Value>,
}

pub struct RestClient {
    client: Client,
    base_url: String,
    auth_token: Option<String>,
}

impl RestClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            auth_token: None,
        }
    }

    pub fn with_auth(mut self, token: String) -> Self {
        self.auth_token = Some(token);
        self
    }

    /// Authenticate with a test password and get JWT tokens
    pub async fn login(&mut self, password: &str) -> Result<LoginResponse> {
        let url = format!("{}/login", self.base_url);

        let request = LoginRequest {
            password: Some(password.to_string()),
            refresh_token: None,
        };

        let response = self
            .client
            .post(&url)
            .form(&request)
            .send()
            .await
            .context("Failed to send login request")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Login failed with status: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let login_response: LoginResponse = response
            .json()
            .await
            .context("Failed to parse login response")?;

        // Set the access token for future requests
        self.auth_token = Some(login_response.access_token.clone());

        info!("Successfully authenticated with proxy");
        Ok(login_response)
    }

    /// Get system information from the node
    pub async fn get_system_info(&self) -> Result<SystemInfo> {
        let url = format!("{}/api/system-info", self.base_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to get system info")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "System info request failed with status: {}",
                response.status()
            ));
        }

        let system_info: SystemInfo = response
            .json()
            .await
            .context("Failed to parse system info response")?;

        debug!("Retrieved system info: {:?}", system_info);
        Ok(system_info)
    }

    /// Send a chat message to a channel
    pub async fn send_chat_message(
        &self,
        channel: &str,
        content: &str,
    ) -> Result<SendChatMessageResponse> {
        let url = format!("{}/api/chat/send", self.base_url);

        let request = SendChatMessageRequest {
            channel: channel.to_string(),
            content: content.to_string(),
        };

        let mut req = self.client.post(&url).json(&request);

        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response =
            req.send().await.context("Failed to send chat message")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Send chat message failed with status: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let chat_response: SendChatMessageResponse = response
            .json()
            .await
            .context("Failed to parse chat response")?;

        info!("Sent chat message to channel '{}': {}", channel, content);
        Ok(chat_response)
    }

    /// Check if the proxy is healthy
    pub async fn health_check(&self) -> Result<()> {
        // Use system info endpoint as a health check
        let system_info = self.get_system_info().await?;

        if system_info.node_id.is_empty() {
            return Err(anyhow::anyhow!(
                "Node ID is empty, proxy may not be connected to hooyad"
            ));
        }

        debug!("Health check passed for node: {}", system_info.node_id);
        Ok(())
    }

    /// Wait for the proxy to become healthy
    pub async fn wait_for_health(
        &self,
        timeout: std::time::Duration,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        loop {
            match self.health_check().await {
                Ok(()) => {
                    info!("Proxy is healthy");
                    return Ok(());
                }
                Err(e) => {
                    if start.elapsed() > timeout {
                        return Err(anyhow::anyhow!(
                            "Proxy did not become healthy within timeout: {}",
                            e
                        ));
                    }
                    debug!("Health check failed, retrying: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// Get the base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get current auth token if set
    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    /// Generic POST with JSON payload
    pub async fn post_json<T, R>(&self, path: &str, payload: &T) -> Result<R>
    where
        T: serde::Serialize,
        R: serde::de::DeserializeOwned,
    {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.post(&url).json(payload);

        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response =
            req.send().await.context("Failed to send POST request")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "POST request to {} failed with status: {} - {}",
                path,
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let result =
            response.json().await.context("Failed to parse response")?;
        Ok(result)
    }

    /// Generic PUT with bytes payload
    pub async fn put_bytes<R>(&self, path: &str, data: Vec<u8>) -> Result<R>
    where
        R: serde::de::DeserializeOwned,
    {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.put(&url).body(data);

        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response =
            req.send().await.context("Failed to send PUT request")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "PUT request to {} failed with status: {} - {}",
                path,
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        let result =
            response.json().await.context("Failed to parse response")?;
        Ok(result)
    }

    /// Create and authenticate a RestClient with a proxy password
    pub async fn create_authenticated(
        base_url: String,
        password: &str,
    ) -> Result<Self> {
        let mut client = Self::new(base_url);
        client.login(password).await?;
        Ok(client)
    }

    /// Start an upload session
    pub async fn start_upload(
        &self,
        size: Option<u64>,
        mimetype: Option<String>,
    ) -> Result<StartUploadResponse> {
        #[derive(Serialize)]
        struct StartUploadRequest {
            size: Option<u64>,
            mimetype: Option<String>,
            chunk_size: u32,
        }

        let request = StartUploadRequest {
            size,
            mimetype,
            chunk_size: 1024 * 1024, // 1MB chunks
        };

        self.post_json("/start-upload", &request).await
    }

    /// Upload a chunk of file data
    pub async fn upload_chunk(
        &self,
        upload_id: &str,
        chunk_index: u32,
        data: Vec<u8>,
    ) -> Result<UploadChunkResponse> {
        let path = format!("/upload-chunk/{upload_id}/{chunk_index}");
        self.put_bytes(&path, data).await
    }

    /// Complete an upload with tags
    pub async fn complete_upload(
        &self,
        upload_id: &str,
        tags: Vec<FileTag>,
    ) -> Result<CompleteUploadResponse> {
        #[derive(Serialize)]
        struct CompleteUploadRequest {
            tags: Vec<FileTag>,
        }

        let request = CompleteUploadRequest { tags };
        let path = format!("/complete-upload/{upload_id}");
        self.post_json(&path, &request).await
    }

    /// Get file information by CID - exercises backend file_row, image_row, video_row methods
    pub async fn get_file_info(&self, cid: &str) -> Result<serde_json::Value> {
        let url = format!("{}/cid-info/{}", self.base_url, cid);

        let mut req = self.client.get(&url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().await.context("Failed to get file info")?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Get file info failed with status: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        response
            .json()
            .await
            .context("Failed to parse file info response")
    }

    /// Wait for file info to become available, polling until timeout
    pub async fn wait_for_file_info(
        &self,
        cid: &str,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let start = Instant::now();
        let mut last_err: Option<anyhow::Error> = None;
        loop {
            match self.get_file_info(cid).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                    if start.elapsed() >= timeout {
                        return Err(last_err.unwrap());
                    }
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            }
        }
    }

    /// Get file tags by CID - exercises backend file_tags method
    pub async fn get_file_tags(&self, cid: &str) -> Result<Vec<FileTag>> {
        let url = format!("{}/cid-tags/{}", self.base_url, cid);

        let mut req = self.client.get(&url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().await.context("Failed to get file tags")?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Get file tags failed with status: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        response
            .json()
            .await
            .context("Failed to parse file tags response")
    }

    /// List files with pagination - exercises backend files_page method
    pub async fn list_files(
        &self,
        page_token: u32,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/all-files/{}", self.base_url, page_token);

        let mut req = self.client.get(&url);
        if let Some(token) = &self.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        let response = req.send().await.context("Failed to list files")?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "List files failed with status: {} - {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ));
        }

        response
            .json()
            .await
            .context("Failed to parse files list response")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rest_client_creation() {
        let client = RestClient::new("http://localhost:8532".to_string());
        assert_eq!(client.base_url(), "http://localhost:8532");
        assert!(client.auth_token().is_none());
    }

    #[test]
    fn test_rest_client_with_auth() {
        let client = RestClient::new("http://localhost:8532".to_string())
            .with_auth("test_token".to_string());
        assert_eq!(client.auth_token().unwrap(), "test_token");
    }

    #[tokio::test]
    async fn test_send_chat_message_request_serialization() {
        let request = SendChatMessageRequest {
            channel: "general".to_string(),
            content: "hello world".to_string(),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"channel\":\"general\""));
        assert!(json.contains("\"content\":\"hello world\""));
    }
}
