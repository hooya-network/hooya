use once_cell::sync::Lazy;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_HOOYAD_ENDPOINT: &str = "127.0.0.1:8531";

pub static DEFAULT_DATA_DIR: Lazy<String> = Lazy::new(|| {
    user_dirs::data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("hooya")
        .to_string_lossy()
        .to_string()
});

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub data_dir: PathBuf,
}

impl RuntimeConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    pub fn ensure_data_dir(&self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.data_dir.exists() {
            fs::create_dir_all(&self.data_dir)?;
        }
        Ok(())
    }

    pub fn ensure_filestore_structure(
        &self,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_data_dir()?;
        fs::create_dir_all(self.data_dir.join("store"))?;
        fs::create_dir_all(self.data_dir.join("forgotten"))?;
        fs::create_dir_all(self.data_dir.join("thumbs"))?;
        fs::create_dir_all(self.data_dir.join("tmp"))?;
        Ok(())
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.data_dir.join("hooya.sqlite")
    }

    pub fn sqlite_uri(&self) -> String {
        format!("sqlite://{}", self.sqlite_path().to_string_lossy())
    }

    pub fn web_password_hash_path(&self) -> PathBuf {
        self.data_dir.join("web-passwd")
    }

    pub fn store_password_hash(
        &self,
        password: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use bcrypt::{hash, DEFAULT_COST};
        self.ensure_data_dir()?;
        let hashed = hash(password, DEFAULT_COST)?;
        fs::write(self.web_password_hash_path(), hashed)?;
        Ok(())
    }

    pub fn load_password_hash(
        &self,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let hash_content = fs::read_to_string(self.web_password_hash_path())?;
        Ok(hash_content.trim().to_string())
    }

    pub fn ensure_password_exists(
        &self,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let hash_file = self.web_password_hash_path();

        if hash_file.exists() {
            return Ok(None); // Password already exists
        }

        // Generate new password
        use rand::Rng;
        const CHARSET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = rand::thread_rng();
        let password: String = (0..16)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect();

        self.store_password_hash(&password)?;
        Ok(Some(password))
    }
}
