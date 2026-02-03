//! FreeBSD jail bootstrapping module
//!
//! This module handles downloading and extracting FreeBSD base systems
//! to bootstrap jails with a complete FreeBSD installation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::StreamExt;
use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::info;

/// Bootstrap configuration options
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BootstrapConfig {
    /// FreeBSD version (e.g., "15.0-RELEASE"). If None, auto-detected from host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Architecture (e.g., "amd64", "aarch64"). If None, auto-detected from host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,

    /// Custom mirror URL. If None, uses official FreeBSD mirrors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,

    /// Force re-download even if cached
    #[serde(default)]
    pub no_cache: bool,

    /// Custom configuration file overrides (path -> content)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_overrides: Option<HashMap<String, String>>,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            version: None,
            architecture: None,
            mirror: None,
            no_cache: false,
            config_overrides: None,
        }
    }
}

/// Bootstrap progress updates
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BootstrapProgress {
    /// Current status of the bootstrap operation
    pub status: BootstrapStatus,

    /// Progress percentage (0-100)
    pub progress: u8,

    /// Human-readable description of current step
    pub current_step: String,

    /// FreeBSD version being bootstrapped
    pub version: String,

    /// Architecture being bootstrapped
    pub architecture: String,
}

/// Bootstrap status
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapStatus {
    /// Bootstrap is initializing
    Initializing,
    /// Downloading base.txz
    Downloading,
    /// Verifying checksum
    Verifying,
    /// Extracting tarball
    Extracting,
    /// Creating configuration files
    Configuring,
    /// Bootstrap completed successfully
    Complete,
    /// Bootstrap failed
    Failed(String),
}

/// Bootstrap errors
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("Insufficient disk space: required {required} bytes, available {available} bytes")]
    DiskSpaceInsufficient { required: u64, available: u64 },

    #[error("Extraction failed: {0}")]
    ExtractionFailed(String),

    #[error("Jail already bootstrapped: {0}")]
    JailAlreadyBootstrapped(String),

    #[error("Invalid version: {0}")]
    InvalidVersion(String),

    #[error("Invalid architecture: {0}")]
    InvalidArchitecture(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Database error: {0}")]
    Database(String),
}

/// Bootstrap cache for storing downloaded tarballs
pub struct BootstrapCache {
    cache_dir: PathBuf,
    db_path: PathBuf,
    max_size_bytes: u64,
}

impl BootstrapCache {
    /// Create a new bootstrap cache
    pub fn new(cache_dir: impl AsRef<Path>) -> Result<Self, BootstrapError> {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        let db_path = cache_dir.join("cache.db");

        // Create cache directory synchronously
        std::fs::create_dir_all(&cache_dir)?;

        Ok(Self {
            cache_dir,
            db_path,
            max_size_bytes: 2 * 1024 * 1024 * 1024, // 2GB default
        })
    }

    /// Create a bootstrap cache with default path
    pub fn with_default_path() -> Result<Self, BootstrapError> {
        Self::new("/var/cache/kawakaze")
    }

    /// Get a cached tarball path
    pub fn get(&self, key: &str) -> Option<PathBuf> {
        let path = self.cache_dir.join(key).join("base.txz");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Store a tarball in the cache
    pub async fn put(&self, key: &str, tarball_path: &Path) -> Result<(), BootstrapError> {
        let cache_entry = self.cache_dir.join(key);
        fs::create_dir_all(&cache_entry).await?;

        let cached_path = cache_entry.join("base.txz");
        fs::copy(tarball_path, &cached_path).await?;

        // Evict if needed
        self.evict_if_needed().await?;

        Ok(())
    }

    /// Evict old entries if cache is too large
    async fn evict_if_needed(&self) -> Result<(), BootstrapError> {
        let total_size = self.calculate_cache_size().await?;

        if total_size > self.max_size_bytes {
            // TODO: Implement LRU eviction
            info!("Cache size {} exceeds limit {}, eviction not yet implemented",
                  total_size, self.max_size_bytes);
        }

        Ok(())
    }

    /// Calculate total cache size
    async fn calculate_cache_size(&self) -> Result<u64, BootstrapError> {
        let mut total = 0u64;

        let mut entries = fs::read_dir(&self.cache_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().is_dir() {
                let tarball_path = entry.path().join("base.txz");
                if tarball_path.exists() {
                    let metadata = fs::metadata(&tarball_path).await?;
                    total += metadata.len();
                }
            }
        }

        Ok(total)
    }

    /// Invalidate a cache entry
    pub async fn invalidate(&self, key: &str) -> Result<(), BootstrapError> {
        let cache_entry = self.cache_dir.join(key);
        if cache_entry.exists() {
            fs::remove_dir_all(cache_entry).await?;
        }
        Ok(())
    }
}

/// Main bootstrap logic
pub struct Bootstrap {
    jail_path: PathBuf,
    config: BootstrapConfig,
    cache: Option<BootstrapCache>,
    progress_tx: mpsc::Sender<BootstrapProgress>,
}

impl Bootstrap {
    /// Create a new bootstrap instance
    pub fn new(
        jail_path: impl AsRef<Path>,
        config: BootstrapConfig,
        progress_tx: mpsc::Sender<BootstrapProgress>,
    ) -> Result<Self, BootstrapError> {
        Ok(Self {
            jail_path: jail_path.as_ref().to_path_buf(),
            config,
            cache: BootstrapCache::with_default_path().ok(),
            progress_tx,
        })
    }

    /// Check if a jail is already bootstrapped
    pub fn is_bootstrapped(jail_path: impl AsRef<Path>) -> bool {
        jail_path.as_ref().join("bin/sh").exists()
    }

    /// Run the bootstrap process
    pub async fn run(mut self) -> Result<(), BootstrapError> {
        // Check if already bootstrapped
        if Self::is_bootstrapped(&self.jail_path) {
            return Err(BootstrapError::JailAlreadyBootstrapped(
                self.jail_path.display().to_string(),
            ));
        }

        // Detect version and architecture
        let version = self.detect_version()?;
        let architecture = self.detect_architecture()?;

        self.report_progress(
            BootstrapStatus::Initializing,
            0,
            "Initializing bootstrap...",
        );

        // Build cache key
        let cache_key = format!("{}-{}", version, architecture);

        // Check cache first
        let tarball_path = if !self.config.no_cache {
            if let Some(ref cache) = self.cache {
                if let Some(cached) = cache.get(&cache_key) {
                    info!("Using cached tarball: {:?}", cached);
                    cached
                } else {
                    self.download_and_verify(&version, &architecture).await?
                }
            } else {
                self.download_and_verify(&version, &architecture).await?
            }
        } else {
            self.download_and_verify(&version, &architecture).await?
        };

        // Store in cache
        if !self.config.no_cache {
            if let Some(ref cache) = self.cache {
                let _ = cache.put(&cache_key, &tarball_path).await;
            }
        }

        // Extract tarball
        self.extract_tarball(&tarball_path).await?;

        // Create configuration files
        self.create_config_files().await?;

        // Report completion
        self.report_progress(
            BootstrapStatus::Complete,
            100,
            "Bootstrap completed successfully",
        );

        Ok(())
    }

    /// Detect FreeBSD version
    fn detect_version(&self) -> Result<String, BootstrapError> {
        if let Some(ref version) = self.config.version {
            return Ok(version.clone());
        }

        // Try to detect from host system
        #[cfg(target_os = "freebsd")]
        {
            use std::ffi::CStr;
            let mut utsname: libc::utsname = unsafe { std::mem::zeroed() };

            if unsafe { libc::uname(&mut utsname) } == 0 {
                let release = unsafe { CStr::from_ptr(utsname.release.as_ptr()) };
                if let Ok(s) = release.to_str() {
                    // Convert "15.0" to "15.0-RELEASE"
                    return Ok(format!("{}-RELEASE", s));
                }
            }
        }

        // Fallback to default
        Ok("15.0-RELEASE".to_string())
    }

    /// Detect system architecture
    fn detect_architecture(&self) -> Result<String, BootstrapError> {
        if let Some(ref arch) = self.config.architecture {
            return Ok(arch.clone());
        }

        // Try to detect from host system
        #[cfg(target_os = "freebsd")]
        {
            use std::ffi::CStr;
            let mut utsname: libc::utsname = unsafe { std::mem::zeroed() };

            if unsafe { libc::uname(&mut utsname) } == 0 {
                let machine = unsafe { CStr::from_ptr(utsname.machine.as_ptr()) };
                if let Ok(s) = machine.to_str() {
                    // Map FreeBSD machine names to architecture names
                    return match s {
                        "amd64" => Ok("amd64".to_string()),
                        "i386" => Ok("i386".to_string()),
                        "aarch64" => Ok("arm64".to_string()),
                        "arm64" => Ok("aarch64".to_string()),
                        _ => Ok(s.to_string()),
                    };
                }
            }
        }

        // Fallback to common architectures
        #[cfg(target_arch = "x86_64")]
        return Ok("amd64".to_string());

        #[cfg(target_arch = "aarch64")]
        return Ok("aarch64".to_string());

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        Err(BootstrapError::InvalidArchitecture(
            "Could not detect architecture".to_string(),
        ))
    }

    /// Build the mirror URL for downloading
    fn build_mirror_url(&self, version: &str, architecture: &str, file: &str) -> String {
        let mirror = self.config.mirror.as_deref()
            .unwrap_or("https://download.freebsd.org/releases");

        // Map architecture for URL (amd64 -> amd64/amd64, aarch64 -> arm64/aarch64)
        let arch_path = match architecture {
            "amd64" => "amd64/amd64",
            "i386" => "i386/i386",
            "aarch64" => "arm64/aarch64",
            "arm64" => "arm64/aarch64",
            _ => architecture,
        };

        format!("{}/{}/{}/{}", mirror, arch_path, version, file)
    }

    /// Download and verify the base.txz tarball
    async fn download_and_verify(
        &mut self,
        version: &str,
        architecture: &str,
    ) -> Result<PathBuf, BootstrapError> {
        let tarball_url = self.build_mirror_url(version, architecture, "base.txz");
        let checksum_url = self.build_mirror_url(version, architecture, "base.txz.sha256");

        info!("Downloading from: {}", tarball_url);

        self.report_progress(
            BootstrapStatus::Downloading,
            10,
            &format!("Downloading FreeBSD base system for {} ({})", version, architecture),
        );

        // Download tarball
        let tarball_path = self.download_file(&tarball_url).await?;

        self.report_progress(
            BootstrapStatus::Verifying,
            50,
            "Verifying checksum...",
        );

        // Download checksum
        let expected_checksum = self.download_checksum(&checksum_url).await?;

        // Verify checksum
        self.verify_checksum(&tarball_path, &expected_checksum).await?;

        self.report_progress(
            BootstrapStatus::Verifying,
            60,
            "Checksum verified",
        );

        Ok(tarball_path)
    }

    /// Download a file with progress tracking
    async fn download_file(&self, url: &str) -> Result<PathBuf, BootstrapError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(600))
            .build()?;

        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(BootstrapError::DownloadFailed(format!(
                "HTTP {}: {}",
                response.status(),
                url
            )));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded = 0u64;

        // Create temp file
        let temp_path = self.jail_path.join(".bootstrap_temp");
        let mut file = File::create(&temp_path).await?;

        // Download with streaming
        let mut stream = response.bytes_stream();

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            // Update progress
            if total_size > 0 {
                let progress = 10 + (downloaded * 35 / total_size) as u8; // 10-45%
                self.report_progress(
                    BootstrapStatus::Downloading,
                    progress,
                    &format!("Downloading... ({}/{})",
                             bytes_to_mb(downloaded),
                             bytes_to_mb(total_size)),
                );
            }
        }

        file.flush().await?;

        Ok(temp_path)
    }

    /// Download checksum file
    async fn download_checksum(&self, url: &str) -> Result<String, BootstrapError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(BootstrapError::DownloadFailed(format!(
                "Failed to download checksum: HTTP {}",
                response.status()
            )));
        }

        let content = response.text().await?;

        // Parse checksum (format: "HASH  base.txz")
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.is_empty() {
            return Err(BootstrapError::DownloadFailed(
                "Invalid checksum format".to_string(),
            ));
        }

        Ok(parts[0].to_string())
    }

    /// Verify SHA256 checksum
    async fn verify_checksum(
        &self,
        path: &Path,
        expected: &str,
    ) -> Result<(), BootstrapError> {
        let contents = fs::read(path).await?;
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        let result = hasher.finalize();
        let actual = hex::encode(result);

        if actual.eq_ignore_ascii_case(expected) {
            Ok(())
        } else {
            Err(BootstrapError::ChecksumMismatch {
                expected: expected.to_string(),
                actual,
            })
        }
    }

    /// Extract the base.txz tarball
    async fn extract_tarball(&mut self, tarball_path: &Path) -> Result<(), BootstrapError> {
        info!("Extracting tarball to: {:?}", self.jail_path);

        self.report_progress(
            BootstrapStatus::Extracting,
            65,
            "Extracting FreeBSD base system...",
        );

        // Use blocking task for tar extraction since we need sync xz2 + tar
        let tarball_path = tarball_path.to_path_buf();
        let jail_path = self.jail_path.clone();

        tokio::task::spawn_blocking(move || {
            // Open the compressed file
            let file = std::fs::File::open(&tarball_path)?;

            // Decompress xz
            let decompressor = xz2::read::XzDecoder::new(file);

            // Extract tar
            let mut archive = tar::Archive::new(decompressor);
            archive.unpack(&jail_path)?;

            Ok::<(), BootstrapError>(())
        })
        .await
        .map_err(|e| BootstrapError::ExtractionFailed(format!("Join error: {}", e)))??;

        self.report_progress(
            BootstrapStatus::Extracting,
            90,
            "Extraction complete",
        );

        Ok(())
    }

    /// Create configuration files
    async fn create_config_files(&self) -> Result<(), BootstrapError> {
        self.report_progress(
            BootstrapStatus::Configuring,
            92,
            "Creating configuration files...",
        );

        let etc_dir = self.jail_path.join("etc");
        fs::create_dir_all(&etc_dir).await?;

        // Apply custom overrides if provided
        if let Some(ref overrides) = self.config.config_overrides {
            for (path, content) in overrides {
                let full_path = self.jail_path.join(path);
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::write(&full_path, content).await?;
            }
        } else {
            // Create default config files
            self.create_default_rc_conf(&etc_dir).await?;
            self.create_default_resolv_conf(&etc_dir).await?;
            self.create_default_hosts(&etc_dir).await?;
        }

        self.report_progress(
            BootstrapStatus::Configuring,
            98,
            "Configuration files created",
        );

        Ok(())
    }

    /// Create default rc.conf
    async fn create_default_rc_conf(&self, etc_dir: &Path) -> Result<(), BootstrapError> {
        let content = format!(
            r#"# Basic RC configuration for {jail_name}
sendmail_enable="NO"
sendmail_submit_enable="NO"
sendmail_outbound_enable="NO"
sshd_enable="YES"
cron_enable="YES"
"#,
            jail_name = self.jail_path.file_name().unwrap_or_default().to_string_lossy()
        );

        fs::write(etc_dir.join("rc.conf"), content).await?;
        Ok(())
    }

    /// Create default resolv.conf
    async fn create_default_resolv_conf(&self, etc_dir: &Path) -> Result<(), BootstrapError> {
        let content = r#"nameserver 1.1.1.1
nameserver 8.8.8.8
"#;

        fs::write(etc_dir.join("resolv.conf"), content).await?;
        Ok(())
    }

    /// Create default hosts file
    async fn create_default_hosts(&self, etc_dir: &Path) -> Result<(), BootstrapError> {
        let hostname = self.jail_path.file_name().unwrap_or_default().to_string_lossy();
        let content = format!(
            r#"127.0.0.1 localhost localhost.localdomain {}
::1 localhost localhost.localdomain
"#,
            hostname
        );

        fs::write(etc_dir.join("hosts"), content).await?;
        Ok(())
    }

    /// Report progress updates
    fn report_progress(&self, status: BootstrapStatus, progress: u8, step: &str) {
        let version = self.config.version.clone().unwrap_or_else(|| "unknown".to_string());
        let architecture = self.config.architecture.clone().unwrap_or_else(|| "unknown".to_string());

        let progress_msg = BootstrapProgress {
            status: status.clone(),
            progress,
            current_step: step.to_string(),
            version: version.clone(),
            architecture: architecture.clone(),
        };

        // Ignore send errors - receiver may be dropped
        let _ = self.progress_tx.try_send(progress_msg);

        info!("[{:?}] {}% - {}", status, progress, step);
    }
}

/// Convert bytes to megabytes
fn bytes_to_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bootstrap_config_default() {
        let config = BootstrapConfig::default();
        assert!(config.version.is_none());
        assert!(config.architecture.is_none());
        assert!(config.mirror.is_none());
        assert!(!config.no_cache);
        assert!(config.config_overrides.is_none());
    }

    #[test]
    fn test_build_mirror_url_default() {
        let bootstrap = create_test_bootstrap();
        let url = bootstrap.build_mirror_url("15.0-RELEASE", "amd64", "base.txz");
        assert_eq!(url, "https://download.freebsd.org/releases/amd64/amd64/15.0-RELEASE/base.txz");
    }

    #[test]
    fn test_build_mirror_url_custom() {
        let mut config = BootstrapConfig::default();
        config.mirror = Some("https://mirror.example.com".to_string());

        let bootstrap = Bootstrap::new("/tmp/test", config, mpsc::channel(1).0).unwrap();
        let url = bootstrap.build_mirror_url("15.0-RELEASE", "amd64", "base.txz");
        assert_eq!(url, "https://mirror.example.com/amd64/amd64/15.0-RELEASE/base.txz");
    }

    #[test]
    fn test_build_mirror_url_aarch64() {
        let bootstrap = create_test_bootstrap();
        let url = bootstrap.build_mirror_url("15.0-RELEASE", "aarch64", "base.txz");
        assert_eq!(url, "https://download.freebsd.org/releases/arm64/aarch64/15.0-RELEASE/base.txz");
    }

    #[test]
    fn test_bytes_to_mb() {
        assert_eq!(bytes_to_mb(0), "0.0 MB");
        assert_eq!(bytes_to_mb(1024 * 1024), "1.0 MB");
        assert_eq!(bytes_to_mb(1536 * 1024), "1.5 MB");
    }

    #[test]
    fn test_detect_version_override() {
        let mut config = BootstrapConfig::default();
        config.version = Some("13.4-RELEASE".to_string());

        let bootstrap = Bootstrap::new("/tmp/test", config, mpsc::channel(1).0).unwrap();
        let version = bootstrap.detect_version().unwrap();
        assert_eq!(version, "13.4-RELEASE");
    }

    #[test]
    fn test_detect_architecture_override() {
        let mut config = BootstrapConfig::default();
        config.architecture = Some("i386".to_string());

        let bootstrap = Bootstrap::new("/tmp/test", config, mpsc::channel(1).0).unwrap();
        let arch = bootstrap.detect_architecture().unwrap();
        assert_eq!(arch, "i386");
    }

    #[test]
    fn test_bootstrap_status_serialization() {
        let status = BootstrapStatus::Downloading;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"downloading\"");
    }

    #[test]
    fn test_bootstrap_progress_serialization() {
        let progress = BootstrapProgress {
            status: BootstrapStatus::Downloading,
            progress: 50,
            current_step: "Downloading...".to_string(),
            version: "15.0-RELEASE".to_string(),
            architecture: "amd64".to_string(),
        };

        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"downloading\""));
        assert!(json.contains(",50,") || json.contains(":50,") || json.contains(":50}"));
    }

    fn create_test_bootstrap() -> Bootstrap {
        let config = BootstrapConfig::default();
        Bootstrap::new("/tmp/test", config, mpsc::channel(1).0).unwrap()
    }
}
