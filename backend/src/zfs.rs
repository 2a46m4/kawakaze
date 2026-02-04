//! ZFS wrapper for managing FreeBSD ZFS datasets, snapshots, and clones
//!
//! This module provides a high-level interface to ZFS operations using the
//! zfs and zpool command-line utilities. It supports creating and managing
//! datasets, snapshots, and clones which are used for jail images and containers.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::string::FromUtf8Error;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ZfsError>;

/// Errors that can occur during ZFS operations
#[derive(Debug, Error)]
pub enum ZfsError {
    #[error("ZFS command failed: {0}")]
    CommandFailed(String),

    #[error("Dataset not found: {0}")]
    DatasetNotFound(String),

    #[error("Dataset already exists: {0}")]
    DatasetExists(String),

    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("Invalid dataset path: {0}")]
    InvalidPath(String),

    #[error("Invalid snapshot name: {0}")]
    InvalidSnapshot(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("UTF-8 conversion error: {0}")]
    Utf8Error(#[from] FromUtf8Error),
}

/// ZFS wrapper for managing datasets, snapshots, and clones
///
/// # Example
///
/// ```no_run
/// use kawakaze_backend::zfs::Zfs;
///
/// // Create a ZFS wrapper for the "tank" pool
/// let zfs = Zfs::new("tank").unwrap();
///
/// // Create a dataset for a jail
/// zfs.create_dataset("tank/jails/webserver").unwrap();
///
/// // Create a snapshot
/// zfs.create_snapshot("tank/jails/webserver", "initial").unwrap();
///
/// // Clone the snapshot to create a new jail
/// zfs.clone_snapshot("tank/jails/webserver@initial", "tank/jails/webserver-clone").unwrap();
/// ```
#[derive(Debug)]
pub struct Zfs {
    pool: String,
}

impl Zfs {
    /// Create a new ZFS wrapper for the given pool
    ///
    /// # Arguments
    ///
    /// * `pool` - The name of the ZFS pool (e.g., "tank", "zroot")
    ///
    /// # Returns
    ///
    /// Returns a `Zfs` instance if the pool exists, otherwise an error
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// let zfs = Zfs::new("tank").unwrap();
    /// let zfs_with_dataset = Zfs::new("tank/kawakaze").unwrap();
    /// ```
    pub fn new(pool_or_dataset: impl AsRef<str>) -> Result<Self> {
        let pool_or_dataset = pool_or_dataset.as_ref();

        // Extract pool name from dataset path (e.g., "tank/kawakaze" -> "tank")
        let pool = pool_or_dataset
            .split('/')
            .next()
            .ok_or_else(|| {
                ZfsError::InvalidPath("Empty pool/dataset path".to_string())
            })?
            .to_string();

        // Verify pool exists
        let output = Command::new("zpool")
            .arg("list")
            .arg("-o")
            .arg("name")
            .arg("-H")
            .output()?;

        if !output.status.success() {
            return Err(ZfsError::CommandFailed(
                "Failed to list ZFS pools".to_string(),
            ));
        }

        let pools = String::from_utf8_lossy(&output.stdout);
        if !pools.lines().any(|l| l == pool) {
            return Err(ZfsError::DatasetNotFound(format!(
                "Pool '{}' not found",
                pool
            )));
        }

        Ok(Zfs { pool })
    }

    /// Get the name of the ZFS pool
    pub fn pool(&self) -> &str {
        &self.pool
    }

    /// Create a new ZFS dataset
    ///
    /// # Arguments
    ///
    /// * `path` - The full dataset path (e.g., "tank/jails/webserver")
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// zfs.create_dataset("tank/jails/webserver").unwrap();
    /// ```
    pub fn create_dataset(&self, path: &str) -> Result<()> {
        if self.dataset_exists(path) {
            return Err(ZfsError::DatasetExists(path.to_string()));
        }

        let output = Command::new("zfs")
            .arg("create")
            .arg("-p")
            .arg("-o")
            .arg("canmount=off")
            .arg(path)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to create dataset '{}': {}",
                path, error_msg
            )));
        }

        Ok(())
    }

    /// Mount a dataset to a specific mountpoint
    pub fn mount_dataset(&self, dataset: &str, mountpoint: &Path) -> Result<()> {
        let mountpoint_str = mountpoint.to_str()
            .ok_or_else(|| ZfsError::CommandFailed("Invalid mountpoint path".to_string()))?;

        // Set canmount to noauto so we can control mounting
        let output = Command::new("zfs")
            .arg("set")
            .arg("canmount=noauto")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to set canmount for '{}': {}",
                dataset, error_msg
            )));
        }

        let output = Command::new("zfs")
            .arg("set")
            .arg(format!("mountpoint={}", mountpoint_str))
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to set mountpoint for '{}': {}",
                dataset, error_msg
            )));
        }

        // Ensure the mountpoint directory exists
        std::fs::create_dir_all(mountpoint).map_err(|e| ZfsError::CommandFailed(format!("Failed to create mountpoint directory: {}", e)))?;

        // Mount the dataset (ignore error if already mounted)
        let output = Command::new("zfs")
            .arg("mount")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            // If it's already mounted, that's okay
            if !error_msg.contains("already mounted") {
                return Err(ZfsError::CommandFailed(format!(
                    "Failed to mount '{}': {}",
                    dataset, error_msg
                )));
            }
        }

        Ok(())
    }

    /// Unmount a dataset
    pub fn unmount_dataset(&self, dataset: &str) -> Result<()> {
        // Try force unmount first to handle busy filesystems
        let output = Command::new("zfs")
            .arg("unmount")
            .arg("-f")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to unmount '{}': {}",
                dataset, error_msg
            )));
        }

        // Reset mountpoint to none
        let output = Command::new("zfs")
            .arg("set")
            .arg("mountpoint=none")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to reset mountpoint for '{}': {}",
                dataset, error_msg
            )));
        }

        // Reset canmount to off
        let output = Command::new("zfs")
            .arg("set")
            .arg("canmount=off")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to reset canmount for '{}': {}",
                dataset, error_msg
            )));
        }

        Ok(())
    }

    /// Check if a dataset is currently mounted
    fn is_dataset_mounted(&self, dataset: &str) -> bool {
        let output = Command::new("zfs")
            .arg("list")
            .arg("-H")
            .arg("-o")
            .arg("mounted")
            .arg(dataset)
            .output();

        match output {
            Ok(out) => {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    stdout.trim() == "yes"
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }

    /// Create a ZFS snapshot of a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset to snapshot (e.g., "tank/jails/webserver")
    /// * `name` - The snapshot name (e.g., "initial", "before-update")
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// zfs.create_snapshot("tank/jails/webserver", "initial").unwrap();
    /// ```
    pub fn create_snapshot(&self, dataset: &str, name: &str) -> Result<()> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let snapshot = format!("{}@{}", dataset, name);

        let output = Command::new("zfs")
            .arg("snapshot")
            .arg(&snapshot)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to create snapshot '{}': {}",
                snapshot, error_msg
            )));
        }

        Ok(())
    }

    /// Clone a ZFS snapshot to create a new dataset
    ///
    /// # Arguments
    ///
    /// * `snapshot` - The full snapshot path (e.g., "tank/jails/webserver@initial")
    /// * `target` - The target dataset path (e.g., "tank/jails/webserver-clone")
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// zfs.clone_snapshot(
    ///     "tank/jails/webserver@initial",
    ///     "tank/jails/webserver-clone"
    /// ).unwrap();
    /// ```
    pub fn clone_snapshot(&self, snapshot: &str, target: &str) -> Result<()> {
        if !self.snapshot_exists(snapshot) {
            return Err(ZfsError::SnapshotNotFound(snapshot.to_string()));
        }

        if self.dataset_exists(target) {
            return Err(ZfsError::DatasetExists(target.to_string()));
        }

        let output = Command::new("zfs")
            .arg("clone")
            .arg(snapshot)
            .arg(target)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to clone snapshot '{}' to '{}': {}",
                snapshot, target, error_msg
            )));
        }

        Ok(())
    }

    /// Destroy a ZFS dataset or snapshot
    ///
    /// # Arguments
    ///
    /// * `path` - The dataset or snapshot path to destroy
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// // Destroy a dataset
    /// zfs.destroy("tank/jails/webserver").unwrap();
    ///
    /// // Destroy a snapshot
    /// zfs.destroy("tank/jails/webserver@initial").unwrap();
    /// ```
    pub fn destroy(&self, path: &str) -> Result<()> {
        let output = Command::new("zfs")
            .arg("destroy")
            .arg("-r")
            .arg(path)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);

            // Check if it's a "dataset does not exist" error
            if error_msg.contains("does not exist") || error_msg.contains("dataset does not exist") {
                return Err(ZfsError::DatasetNotFound(path.to_string()));
            }

            return Err(ZfsError::CommandFailed(format!(
                "Failed to destroy '{}': {}",
                path, error_msg
            )));
        }

        Ok(())
    }

    /// Get the mountpoint of a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path (e.g., "tank/jails/webserver")
    ///
    /// # Returns
    ///
    /// Returns the mountpoint as a `PathBuf`
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// let mountpoint = zfs.get_mountpoint("tank/jails/webserver").unwrap();
    /// println!("Jail mounted at: {:?}", mountpoint);
    /// ```
    pub fn get_mountpoint(&self, dataset: &str) -> Result<PathBuf> {
        let output = Command::new("zfs")
            .arg("list")
            .arg("-o")
            .arg("mountpoint")
            .arg("-H")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to get mountpoint for '{}': {}",
                dataset, error_msg
            )));
        }

        let mountpoint = String::from_utf8(output.stdout)?
            .trim()
            .to_string();

        Ok(PathBuf::from(mountpoint))
    }

    /// List all snapshots for a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path (e.g., "tank/jails/webserver")
    ///
    /// # Returns
    ///
    /// Returns a vector of snapshot names (without the dataset prefix)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// let snapshots = zfs.list_snapshots("tank/jails/webserver").unwrap();
    /// for snap in snapshots {
    ///     println!("Snapshot: {}", snap);
    /// }
    /// ```
    pub fn list_snapshots(&self, dataset: &str) -> Result<Vec<String>> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let output = Command::new("zfs")
            .arg("list")
            .arg("-t")
            .arg("snapshot")
            .arg("-o")
            .arg("name")
            .arg("-H")
            .arg("-s")
            .arg("creation")
            .arg(format!("{}@*", dataset))
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to list snapshots for '{}': {}",
                dataset, error_msg
            )));
        }

        let snapshots = String::from_utf8(output.stdout)?
            .lines()
            .map(|line| {
                // Strip the dataset@ prefix to return just the snapshot name
                line.trim()
                    .strip_prefix(&format!("{}@", dataset))
                    .unwrap_or(line.trim())
                    .to_string()
            })
            .filter(|s| !s.is_empty())
            .collect();

        Ok(snapshots)
    }

    /// Check if a dataset exists
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path to check
    ///
    /// # Returns
    ///
    /// Returns `true` if the dataset exists, `false` otherwise
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// if zfs.dataset_exists("tank/jails/webserver") {
    ///     println!("Dataset exists");
    /// }
    /// ```
    pub fn dataset_exists(&self, dataset: &str) -> bool {
        let output = Command::new("zfs")
            .arg("list")
            .arg("-o")
            .arg("name")
            .arg("-H")
            .arg(dataset)
            .output();

        match output {
            Ok(result) => result.status.success(),
            Err(_) => false,
        }
    }

    /// Check if a snapshot exists
    ///
    /// # Arguments
    ///
    /// * `snapshot` - The full snapshot path (e.g., "tank/jails/webserver@initial")
    ///
    /// # Returns
    ///
    /// Returns `true` if the snapshot exists, `false` otherwise
    pub fn snapshot_exists(&self, snapshot: &str) -> bool {
        let output = Command::new("zfs")
            .arg("list")
            .arg("-t")
            .arg("snapshot")
            .arg("-o")
            .arg("name")
            .arg("-H")
            .arg(snapshot)
            .output();

        match output {
            Ok(result) => result.status.success(),
            Err(_) => false,
        }
    }

    /// Set a ZFS property on a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path
    /// * `prop` - The property name (e.g., "mountpoint", "compression", "quota")
    /// * `value` - The property value
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// // Set compression
    /// zfs.set_property("tank/jails/webserver", "compression", "lz4").unwrap();
    ///
    /// // Set mountpoint
    /// zfs.set_property("tank/jails/webserver", "mountpoint", "/mnt/jails/webserver").unwrap();
    ///
    /// // Set quota (10 GB)
    /// zfs.set_property("tank/jails/webserver", "quota", "10G").unwrap();
    /// ```
    pub fn set_property(&self, dataset: &str, prop: &str, value: &str) -> Result<()> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let output = Command::new("zfs")
            .arg("set")
            .arg(format!("{}={}", prop, value))
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to set property '{}' on '{}': {}",
                prop, dataset, error_msg
            )));
        }

        Ok(())
    }

    /// Get a ZFS property value from a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path
    /// * `prop` - The property name
    ///
    /// # Returns
    ///
    /// Returns the property value as a string
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// let compression = zfs.get_property("tank/jails/webserver", "compression").unwrap();
    /// println!("Compression: {}", compression);
    ///
    /// let mountpoint = zfs.get_property("tank/jails/webserver", "mountpoint").unwrap();
    /// println!("Mountpoint: {}", mountpoint);
    /// ```
    pub fn get_property(&self, dataset: &str, prop: &str) -> Result<String> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let output = Command::new("zfs")
            .arg("get")
            .arg("-H")
            .arg("-o")
            .arg("value")
            .arg(prop)
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to get property '{}' from '{}': {}",
                prop, dataset, error_msg
            )));
        }

        let value = String::from_utf8(output.stdout)?
            .trim()
            .to_string();

        Ok(value)
    }

    /// List all datasets under a path
    ///
    /// # Arguments
    ///
    /// * `path` - The dataset path (e.g., "tank/jails")
    ///
    /// # Returns
    ///
    /// Returns a vector of dataset names
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// let jails = zfs.list_datasets("tank/jails").unwrap();
    /// for jail in jails {
    ///     println!("Jail: {}", jail);
    /// }
    /// ```
    pub fn list_datasets(&self, path: &str) -> Result<Vec<String>> {
        let output = Command::new("zfs")
            .arg("list")
            .arg("-o")
            .arg("name")
            .arg("-H")
            .arg("-r")
            .arg(path)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to list datasets under '{}': {}",
                path, error_msg
            )));
        }

        let datasets = String::from_utf8(output.stdout)?
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(datasets)
    }

    /// Rollback a dataset to a snapshot
    ///
    /// # Warning
    ///
    /// This will destroy all snapshots taken after the target snapshot.
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path
    /// * `snapshot` - The snapshot name to rollback to
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// // Rollback to the "initial" snapshot
    /// zfs.rollback("tank/jails/webserver", "initial").unwrap();
    /// ```
    pub fn rollback(&self, dataset: &str, snapshot: &str) -> Result<()> {
        let snapshot_path = format!("{}@{}", dataset, snapshot);

        if !self.snapshot_exists(&snapshot_path) {
            return Err(ZfsError::SnapshotNotFound(snapshot_path));
        }

        let output = Command::new("zfs")
            .arg("rollback")
            .arg("-r")
            .arg(&snapshot_path)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to rollback to snapshot '{}': {}",
                snapshot_path, error_msg
            )));
        }

        Ok(())
    }

    /// Get the amount of space used by a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path
    ///
    /// # Returns
    ///
    /// Returns the used space in bytes
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// let used = zfs.get_used_space("tank/jails/webserver").unwrap();
    /// println!("Used space: {} bytes", used);
    /// ```
    pub fn get_used_space(&self, dataset: &str) -> Result<u64> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let output = Command::new("zfs")
            .arg("list")
            .arg("-o")
            .arg("used")
            .arg("-H")
            .arg("-p")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to get used space for '{}': {}",
                dataset, error_msg
            )));
        }

        let used_str = String::from_utf8(output.stdout)?
            .trim()
            .to_string();

        let used = used_str.parse::<u64>().map_err(|_| {
            ZfsError::CommandFailed(format!(
                "Failed to parse used space value: {}",
                used_str
            ))
        })?;

        Ok(used)
    }

    /// Get the available space of a dataset
    ///
    /// # Arguments
    ///
    /// * `dataset` - The dataset path
    ///
    /// # Returns
    ///
    /// Returns the available space in bytes
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// let avail = zfs.get_available_space("tank/jails").unwrap();
    /// println!("Available space: {} bytes", avail);
    /// ```
    pub fn get_available_space(&self, dataset: &str) -> Result<u64> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let output = Command::new("zfs")
            .arg("list")
            .arg("-o")
            .arg("avail")
            .arg("-H")
            .arg("-p")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to get available space for '{}': {}",
                dataset, error_msg
            )));
        }

        let avail_str = String::from_utf8(output.stdout)?
            .trim()
            .to_string();

        let avail = avail_str.parse::<u64>().map_err(|_| {
            ZfsError::CommandFailed(format!(
                "Failed to parse available space value: {}",
                avail_str
            ))
        })?;

        Ok(avail)
    }

    /// Promote a clone to a normal dataset
    ///
    /// This breaks the dependency on the origin snapshot, allowing the
    /// origin snapshot to be destroyed if needed.
    ///
    /// # Arguments
    ///
    /// * `dataset` - The cloned dataset to promote
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// // Promote a clone to make it independent
    /// zfs.promote("tank/jails/webserver-clone").unwrap();
    /// ```
    pub fn promote(&self, dataset: &str) -> Result<()> {
        if !self.dataset_exists(dataset) {
            return Err(ZfsError::DatasetNotFound(dataset.to_string()));
        }

        let output = Command::new("zfs")
            .arg("promote")
            .arg(dataset)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to promote dataset '{}': {}",
                dataset, error_msg
            )));
        }

        Ok(())
    }

    /// Rename a dataset
    ///
    /// # Arguments
    ///
    /// * `old_name` - The current dataset name
    /// * `new_name` - The new dataset name
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kawakaze_backend::zfs::Zfs;
    /// # let zfs = Zfs::new("tank").unwrap();
    /// zfs.rename("tank/jails/webserver", "tank/jails/webserver-old").unwrap();
    /// ```
    pub fn rename(&self, old_name: &str, new_name: &str) -> Result<()> {
        if !self.dataset_exists(old_name) {
            return Err(ZfsError::DatasetNotFound(old_name.to_string()));
        }

        if self.dataset_exists(new_name) {
            return Err(ZfsError::DatasetExists(new_name.to_string()));
        }

        let output = Command::new("zfs")
            .arg("rename")
            .arg(old_name)
            .arg(new_name)
            .output()?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            return Err(ZfsError::CommandFailed(format!(
                "Failed to rename dataset from '{}' to '{}': {}",
                old_name, new_name, error_msg
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a running ZFS pool
    // Most will be marked as ignored unless a ZFS pool is available

    #[test]
    fn test_zfs_new_invalid_pool() {
        let result = Zfs::new("nonexistent_pool_test_12345");
        assert!(result.is_err());
        match result.unwrap_err() {
            ZfsError::DatasetNotFound(_) => {},
            _ => panic!("Expected DatasetNotFound error"),
        }
    }

    #[test]
    #[ignore]
    fn test_zfs_new_valid_pool() {
        // This test requires an actual ZFS pool
        // Run with: cargo test -- --ignored
        let zfs_result = Zfs::new("tank");
        let zfs = match zfs_result {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };
        assert_eq!(zfs.pool(), "tank");
    }

    #[test]
    #[ignore]
    fn test_create_and_destroy_dataset() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_create";
        let _ = zfs.destroy(test_dataset); // Clean up from any previous run

        // Create dataset
        let result = zfs.create_dataset(test_dataset);
        assert!(result.is_ok());
        assert!(zfs.dataset_exists(test_dataset));

        // Try to create again (should fail)
        let result = zfs.create_dataset(test_dataset);
        assert!(result.is_err());
        match result.unwrap_err() {
            ZfsError::DatasetExists(_) => {},
            _ => panic!("Expected DatasetExists error"),
        }

        // Destroy dataset
        let result = zfs.destroy(test_dataset);
        assert!(result.is_ok());
        assert!(!zfs.dataset_exists(test_dataset));
    }

    #[test]
    #[ignore]
    fn test_create_and_list_snapshots() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_snapshots";
        let _ = zfs.destroy(test_dataset); // Clean up from any previous run
        zfs.create_dataset(test_dataset).unwrap();

        // Create snapshots
        zfs.create_snapshot(test_dataset, "snap1").unwrap();
        zfs.create_snapshot(test_dataset, "snap2").unwrap();
        zfs.create_snapshot(test_dataset, "snap3").unwrap();

        // List snapshots
        let snapshots = zfs.list_snapshots(test_dataset).unwrap();
        assert_eq!(snapshots.len(), 3);
        assert!(snapshots.contains(&"snap1".to_string()));
        assert!(snapshots.contains(&"snap2".to_string()));
        assert!(snapshots.contains(&"snap3".to_string()));

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
    }

    #[test]
    #[ignore]
    fn test_clone_snapshot() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_clone";
        let test_clone = "tank/kawakaze_test_clone_target";
        let _ = zfs.destroy(test_dataset);
        let _ = zfs.destroy(test_clone);

        zfs.create_dataset(test_dataset).unwrap();
        zfs.create_snapshot(test_dataset, "initial").unwrap();

        // Clone snapshot
        let result = zfs.clone_snapshot(
            &format!("{}@initial", test_dataset),
            test_clone
        );
        assert!(result.is_ok());
        assert!(zfs.dataset_exists(test_clone));

        // Try to clone again (should fail)
        let result = zfs.clone_snapshot(
            &format!("{}@initial", test_dataset),
            test_clone
        );
        assert!(result.is_err());

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
        zfs.destroy(test_clone).unwrap();
    }

    #[test]
    #[ignore]
    fn test_get_mountpoint() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_mountpoint";
        let _ = zfs.destroy(test_dataset);
        zfs.create_dataset(test_dataset).unwrap();

        // Get mountpoint
        let mountpoint = zfs.get_mountpoint(test_dataset).unwrap();
        assert!(mountpoint.is_absolute());

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
    }

    #[test]
    #[ignore]
    fn test_set_and_get_property() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_properties";
        let _ = zfs.destroy(test_dataset);
        zfs.create_dataset(test_dataset).unwrap();

        // Set compression
        zfs.set_property(test_dataset, "compression", "lz4").unwrap();

        // Get compression
        let compression = zfs.get_property(test_dataset, "compression").unwrap();
        assert_eq!(compression, "lz4");

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
    }

    #[test]
    #[ignore]
    fn test_rollback() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_rollback";
        let _ = zfs.destroy(test_dataset);
        zfs.create_dataset(test_dataset).unwrap();

        // Create snapshot
        zfs.create_snapshot(test_dataset, "initial").unwrap();

        // Rollback to snapshot
        let result = zfs.rollback(test_dataset, "initial");
        assert!(result.is_ok());

        // Try to rollback to non-existent snapshot
        let result = zfs.rollback(test_dataset, "nonexistent");
        assert!(result.is_err());

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
    }

    #[test]
    #[ignore]
    fn test_get_space() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_space";
        let _ = zfs.destroy(test_dataset);
        zfs.create_dataset(test_dataset).unwrap();

        // Get used space
        let used = zfs.get_used_space(test_dataset).unwrap();
        assert!(used >= 0);

        // Get available space
        let avail = zfs.get_available_space(test_dataset).unwrap();
        assert!(avail > 0);

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
    }

    #[test]
    #[ignore]
    fn test_promote() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_dataset = "tank/kawakaze_test_promote";
        let test_clone = "tank/kawakaze_test_promote_clone";
        let _ = zfs.destroy(test_dataset);
        let _ = zfs.destroy(test_clone);

        zfs.create_dataset(test_dataset).unwrap();
        zfs.create_snapshot(test_dataset, "initial").unwrap();
        zfs.clone_snapshot(&format!("{}@initial", test_dataset), test_clone).unwrap();

        // Promote clone
        let result = zfs.promote(test_clone);
        assert!(result.is_ok());

        // Cleanup
        zfs.destroy(test_dataset).unwrap();
        zfs.destroy(test_clone).unwrap();
    }

    #[test]
    #[ignore]
    fn test_rename() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let old_name = "tank/kawakaze_test_rename_old";
        let new_name = "tank/kawakaze_test_rename_new";
        let _ = zfs.destroy(old_name);
        let _ = zfs.destroy(new_name);

        zfs.create_dataset(old_name).unwrap();

        // Rename dataset
        let result = zfs.rename(old_name, new_name);
        assert!(result.is_ok());
        assert!(!zfs.dataset_exists(old_name));
        assert!(zfs.dataset_exists(new_name));

        // Try to rename again (should fail - old dataset doesn't exist)
        let result = zfs.rename(old_name, new_name);
        assert!(result.is_err());

        // Cleanup
        zfs.destroy(new_name).unwrap();
    }

    #[test]
    #[ignore]
    fn test_list_datasets() {
        let zfs = match Zfs::new("tank") {
            Ok(z) => z,
            Err(_) => {
                eprintln!("Skipping test: no 'tank' pool available");
                return;
            }
        };

        let test_parent = "tank/kawakaze_test_list";
        let test_child1 = "tank/kawakaze_test_list/child1";
        let test_child2 = "tank/kawakaze_test_list/child2";
        let _ = zfs.destroy(test_parent);

        zfs.create_dataset(test_parent).unwrap();
        zfs.create_dataset(test_child1).unwrap();
        zfs.create_dataset(test_child2).unwrap();

        // List datasets
        let datasets = zfs.list_datasets(test_parent).unwrap();
        assert!(datasets.len() >= 3); // parent + 2 children

        // Cleanup
        zfs.destroy(test_parent).unwrap();
    }
}
