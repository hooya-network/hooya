use once_cell::sync::Lazy;
use rand::RngCore;
use serde::{Deserialize, Serialize};
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

    pub fn jwt_secret_path(&self) -> PathBuf {
        self.data_dir.join("jwt-secret")
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
            return Ok(None); // password already exists
        }

        // generate new password
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

    pub fn store_jwt_secret(
        &self,
        secret: &[u8; 32],
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_data_dir()?;
        fs::write(self.jwt_secret_path(), secret)?;
        Ok(())
    }

    pub fn load_jwt_secret(
        &self,
    ) -> Result<[u8; 32], Box<dyn std::error::Error>> {
        let secret_bytes = fs::read(self.jwt_secret_path())?;
        if secret_bytes.len() != 32 {
            return Err("Invalid JWT secret length".into());
        }
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&secret_bytes);
        Ok(secret)
    }

    pub fn ensure_jwt_secret_exists(
        &self,
    ) -> Result<[u8; 32], Box<dyn std::error::Error>> {
        match self.load_jwt_secret() {
            Ok(secret) => Ok(secret),
            Err(_) => {
                // generate new secret if file doesn't exist or is invalid
                let mut secret = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut secret);
                self.store_jwt_secret(&secret)?;
                Ok(secret)
            }
        }
    }

    pub fn hooya_config_path(&self) -> PathBuf {
        self.data_dir.join("hooya.toml")
    }

    pub fn load_hooya_config(
        &self,
    ) -> Result<HooyaConfig, Box<dyn std::error::Error>> {
        let config_path = self.hooya_config_path();

        if config_path.exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: HooyaConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            // create default config if it doesn't exist
            let default_config = HooyaConfig::default();
            self.save_hooya_config(&default_config)?;
            Ok(default_config)
        }
    }

    pub fn save_hooya_config(
        &self,
        config: &HooyaConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_data_dir()?;
        let toml_content = toml::to_string_pretty(config)?;
        fs::write(self.hooya_config_path(), toml_content)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooyaConfig {
    pub instance: InstanceConfig,
    pub networking: NetworkingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub name: String,
    pub operator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkingConfig {
    pub max_peers: usize,
    pub max_message_size_bytes: usize,
    pub discovery: DiscoveryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub mdns: MdnsConfig,
    pub dns: DnsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdnsConfig {
    pub enabled: bool,
    pub service_name: String,
    pub discovery_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsConfig {
    pub enabled: bool,
    pub bootstrap_domain: String,
    pub lookup_interval_secs: u64,
}

impl Default for HooyaConfig {
    fn default() -> Self {
        Self {
            instance: InstanceConfig {
                name: "hooya".to_string(),
                operator: "anonymous".to_string(),
            },
            networking: NetworkingConfig {
                max_peers: 50,
                max_message_size_bytes: 1024 * 1024, // 1MB
                discovery: DiscoveryConfig {
                    mdns: MdnsConfig {
                        enabled: true,
                        service_name: "hooya-mesh".to_string(),
                        discovery_interval_secs: 30,
                    },
                    dns: DnsConfig {
                        enabled: true,
                        bootstrap_domain: "bootstrap.hooya.org".to_string(),
                        lookup_interval_secs: 300,
                    },
                },
            },
        }
    }
}
