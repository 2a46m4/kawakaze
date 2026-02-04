//! Configuration file loading for Kawakaze
//!
//! This module handles loading and saving configuration from TOML files.
//! It provides default values for all configuration options.

use std::path::{Path, PathBuf};
use std::fs;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, ConfigError>;

/// Errors that can occur during configuration loading or saving
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parsing error: {0}")]
    TomlParse(String),
    #[error("Config file not found")]
    NotFound,
    #[error("Invalid value: {0}")]
    InvalidValue(String),
}

/// Main configuration structure for Kawakaze
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KawakazeConfig {
    /// ZFS pool name for jail storage
    pub zfs_pool: String,
    /// Network configuration
    #[serde(default)]
    pub network: NetworkConfig,
    /// Storage configuration
    #[serde(default)]
    pub storage: StorageConfig,
    /// API configuration
    #[serde(default)]
    pub api: ApiConfig,
}

/// Network configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// CIDR block for container IP allocation
    #[serde(default = "default_container_cidr")]
    pub container_cidr: String,
    /// Bridge device name
    #[serde(default = "default_bridge_name")]
    pub bridge_name: String,
    /// Whether NAT is enabled
    #[serde(default = "default_nat_enabled")]
    pub nat_enabled: bool,
}

/// Storage configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Path to SQLite database
    #[serde(default = "default_database_path")]
    pub database_path: String,
    /// Path to Unix socket
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
    /// Path to cache directory
    #[serde(default = "default_cache_path")]
    pub cache_path: String,
}

/// API configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// API timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

// Default value functions

fn default_container_cidr() -> String {
    "10.11.0.0/16".to_string()
}

fn default_bridge_name() -> String {
    "kawakaze-bridge".to_string()
}

fn default_nat_enabled() -> bool {
    true
}

fn default_database_path() -> String {
    "/var/db/kawakaze/kawakaze.db".to_string()
}

fn default_socket_path() -> String {
    "/var/run/kawakaze.sock".to_string()
}

fn default_cache_path() -> String {
    "/var/cache/kawakaze".to_string()
}

fn default_timeout() -> u64 {
    30
}

// Default implementations

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            container_cidr: default_container_cidr(),
            bridge_name: default_bridge_name(),
            nat_enabled: default_nat_enabled(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database_path: default_database_path(),
            socket_path: default_socket_path(),
            cache_path: default_cache_path(),
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            timeout: default_timeout(),
        }
    }
}

impl KawakazeConfig {
    /// Load configuration from a specific path
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ConfigError::NotFound
                } else {
                    ConfigError::Io(e)
                }
            })?;

        let config: KawakazeConfig = toml::from_str(&contents)
            .map_err(|e| ConfigError::TomlParse(e.to_string()))?;

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Load configuration from default locations
    ///
    /// Searches in the following order:
    /// 1. `/etc/kawakaze/config.toml`
    /// 2. `~/.config/kawakaze/config.toml`
    ///
    /// If neither exists, returns default configuration.
    pub fn load_defaults() -> Result<Self> {
        let system_config = Path::new("/etc/kawakaze/config.toml");
        let user_config = std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(".config/kawakaze/config.toml"))
            .unwrap_or_else(|_| PathBuf::from("~/.config/kawakaze/config.toml"));

        // Try system config first
        if system_config.exists() {
            return Self::load(system_config);
        }

        // Then try user config
        if user_config.exists() {
            return Self::load(user_config);
        }

        // Return default config if none found
        Ok(Self::default())
    }

    /// Save configuration to a specific path
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Serialize to TOML
        let toml_string = toml::to_string_pretty(self)
            .map_err(|e| ConfigError::InvalidValue(e.to_string()))?;

        // Write to file
        fs::write(path, toml_string)?;

        Ok(())
    }

    /// Validate configuration values
    fn validate(&self) -> Result<()> {
        // Validate ZFS pool name
        if self.zfs_pool.is_empty() {
            return Err(ConfigError::InvalidValue("ZFS pool name cannot be empty".to_string()));
        }

        // Validate CIDR format
        if !is_valid_cidr(&self.network.container_cidr) {
            return Err(ConfigError::InvalidValue(format!(
                "Invalid CIDR format: {}",
                self.network.container_cidr
            )));
        }

        // Validate paths are not empty
        if self.storage.database_path.is_empty() {
            return Err(ConfigError::InvalidValue("Database path cannot be empty".to_string()));
        }
        if self.storage.socket_path.is_empty() {
            return Err(ConfigError::InvalidValue("Socket path cannot be empty".to_string()));
        }
        if self.storage.cache_path.is_empty() {
            return Err(ConfigError::InvalidValue("Cache path cannot be empty".to_string()));
        }

        // Validate timeout is reasonable
        if self.api.timeout == 0 {
            return Err(ConfigError::InvalidValue("API timeout cannot be zero".to_string()));
        }
        if self.api.timeout > 3600 {
            return Err(ConfigError::InvalidValue("API timeout cannot exceed 3600 seconds".to_string()));
        }

        Ok(())
    }
}

impl Default for KawakazeConfig {
    fn default() -> Self {
        Self {
            zfs_pool: "zroot/kawakaze".to_string(),
            network: NetworkConfig::default(),
            storage: StorageConfig::default(),
            api: ApiConfig::default(),
        }
    }
}

/// Helper function to validate CIDR notation
fn is_valid_cidr(cidr: &str) -> bool {
    // Basic CIDR validation: IP address / prefix length
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 {
        return false;
    }

    // Validate it's an IP address
    parts[0].parse::<std::net::IpAddr>().is_ok()
    // Validate prefix length
    && parts[1].parse::<u8>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_values() {
        let config = KawakazeConfig::default();

        assert_eq!(config.zfs_pool, "zroot/kawakaze");
        assert_eq!(config.network.container_cidr, "10.11.0.0/16");
        assert_eq!(config.network.bridge_name, "kawakaze-bridge");
        assert_eq!(config.network.nat_enabled, true);
        assert_eq!(config.storage.database_path, "/var/db/kawakaze/kawakaze.db");
        assert_eq!(config.storage.socket_path, "/var/run/kawakaze.sock");
        assert_eq!(config.storage.cache_path, "/var/cache/kawakaze");
        assert_eq!(config.api.timeout, 30);
    }

    #[test]
    fn test_network_config_default() {
        let config = NetworkConfig::default();

        assert_eq!(config.container_cidr, "10.11.0.0/16");
        assert_eq!(config.bridge_name, "kawakaze-bridge");
        assert_eq!(config.nat_enabled, true);
    }

    #[test]
    fn test_storage_config_default() {
        let config = StorageConfig::default();

        assert_eq!(config.database_path, "/var/db/kawakaze/kawakaze.db");
        assert_eq!(config.socket_path, "/var/run/kawakaze.sock");
        assert_eq!(config.cache_path, "/var/cache/kawakaze");
    }

    #[test]
    fn test_api_config_default() {
        let config = ApiConfig::default();

        assert_eq!(config.timeout, 30);
    }

    #[test]
    fn test_load_and_save_config() {
        let config = KawakazeConfig {
            zfs_pool: "myPool/jails".to_string(),
            network: NetworkConfig {
                container_cidr: "192.168.1.0/24".to_string(),
                bridge_name: "my-bridge".to_string(),
                nat_enabled: false,
            },
            storage: StorageConfig {
                database_path: "/tmp/kawakaze.db".to_string(),
                socket_path: "/tmp/kawakaze.sock".to_string(),
                cache_path: "/tmp/cache".to_string(),
            },
            api: ApiConfig {
                timeout: 60,
            },
        };

        // Save to temp file
        let mut temp_file = NamedTempFile::new().unwrap();
        config.save(temp_file.path()).unwrap();

        // Load back
        let loaded = KawakazeConfig::load(temp_file.path()).unwrap();

        assert_eq!(loaded.zfs_pool, "myPool/jails");
        assert_eq!(loaded.network.container_cidr, "192.168.1.0/24");
        assert_eq!(loaded.network.bridge_name, "my-bridge");
        assert_eq!(loaded.network.nat_enabled, false);
        assert_eq!(loaded.storage.database_path, "/tmp/kawakaze.db");
        assert_eq!(loaded.storage.socket_path, "/tmp/kawakaze.sock");
        assert_eq!(loaded.storage.cache_path, "/tmp/cache");
        assert_eq!(loaded.api.timeout, 60);
    }

    #[test]
    fn test_load_minimal_config() {
        let toml_content = r#"
            zfs_pool = "zroot/kawakaze"
        "#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(toml_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let config = KawakazeConfig::load(temp_file.path()).unwrap();

        assert_eq!(config.zfs_pool, "zroot/kawakaze");
        // Rest should be defaults
        assert_eq!(config.network.container_cidr, "10.11.0.0/16");
        assert_eq!(config.network.bridge_name, "kawakaze-bridge");
        assert_eq!(config.storage.database_path, "/var/db/kawakaze/kawakaze.db");
        assert_eq!(config.api.timeout, 30);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = KawakazeConfig::load("/nonexistent/path/config.toml");
        assert!(matches!(result, Err(ConfigError::NotFound)));
    }

    #[test]
    fn test_load_invalid_toml() {
        let invalid_toml = "invalid [ toml content";

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(invalid_toml.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = KawakazeConfig::load(temp_file.path());
        assert!(matches!(result, Err(ConfigError::TomlParse(_))));
    }

    #[test]
    fn test_validate_empty_zfs_pool() {
        let config = KawakazeConfig {
            zfs_pool: "".to_string(),
            ..Default::default()
        };

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    #[test]
    fn test_validate_invalid_cidr() {
        let config = KawakazeConfig {
            zfs_pool: "zroot/kawakaze".to_string(),
            network: NetworkConfig {
                container_cidr: "invalid-cidr".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    #[test]
    fn test_validate_empty_database_path() {
        let config = KawakazeConfig {
            zfs_pool: "zroot/kawakaze".to_string(),
            storage: StorageConfig {
                database_path: "".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    #[test]
    fn test_validate_zero_timeout() {
        let config = KawakazeConfig {
            zfs_pool: "zroot/kawakaze".to_string(),
            api: ApiConfig {
                timeout: 0,
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    #[test]
    fn test_validate_excessive_timeout() {
        let config = KawakazeConfig {
            zfs_pool: "zroot/kawakaze".to_string(),
            api: ApiConfig {
                timeout: 4000,
            },
            ..Default::default()
        };

        let result = config.validate();
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    #[test]
    fn test_validate_valid_config() {
        let config = KawakazeConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_is_valid_cidr() {
        assert!(is_valid_cidr("10.11.0.0/16"));
        assert!(is_valid_cidr("192.168.1.0/24"));
        assert!(is_valid_cidr("172.16.0.0/12"));
        assert!(!is_valid_cidr("invalid"));
        assert!(!is_valid_cidr("10.11.0.0"));
        assert!(!is_valid_cidr("/16"));
        assert!(!is_valid_cidr("10.11.0.0/invalid"));
    }

    #[test]
    fn test_load_defaults_creates_directories() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("subdir/config.toml");

        let config = KawakazeConfig::default();
        config.save(&config_path).unwrap();

        assert!(config_path.exists());
        assert!(config_path.parent().unwrap().exists());
    }
}
