//! Kawakaze backend - Jail management service
//!
//! This module handles the actual management of FreeBSD jails,
//! communicating with clients through a unix socket.

pub mod jail;
pub mod api;
pub mod handler;
pub mod server;
pub mod store;
pub mod bootstrap;

use crate::jail::{Jail, JailError, JailState};
use crate::store::{JailStore, StoreError};
use crate::bootstrap::{BootstrapProgress, BootstrapStatus};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Type for bootstrap progress sender
pub type BootstrapProgressSender = mpsc::Sender<BootstrapProgress>;

/// Jail manager - handles jail lifecycle
pub struct JailManager {
    pub(crate) socket_path: PathBuf,
    pub(crate) jails: HashMap<String, Jail>,
    running: bool,
    store: Option<JailStore>,
    /// Bootstrap progress trackers (jail name -> progress sender)
    pub bootstrap_tracker: HashMap<String, BootstrapProgressSender>,
    /// Bootstrap progress state (jail name -> latest progress)
    pub bootstrap_progress: HashMap<String, BootstrapProgress>,
}

impl JailManager {
    /// Create a new jail manager
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            jails: HashMap::new(),
            running: false,
            store: None,
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
        }
    }

    /// Create a jail manager with default socket path
    pub fn with_default_socket() -> Self {
        Self::new("/var/run/kawakaze.sock")
    }

    /// Create a jail manager with database persistence
    pub fn with_database(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let store = JailStore::new(db_path)?;

        Ok(Self {
            socket_path: PathBuf::from("/var/run/kawakaze.sock"),
            jails: HashMap::new(),
            running: false,
            store: Some(store),
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
        })
    }

    /// Create a jail manager with database persistence at default location
    pub fn with_default_database() -> Result<Self, StoreError> {
        Self::with_database("/var/db/kawakaze.db")
    }

    /// Create a jail manager with custom socket and database paths
    pub fn with_paths(socket_path: impl Into<PathBuf>, db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let store = JailStore::new(db_path)?;

        Ok(Self {
            socket_path: socket_path.into(),
            jails: HashMap::new(),
            running: false,
            store: Some(store),
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
        })
    }

    /// Start the jail manager service
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.running {
            return Err("JailManager is already running".into());
        }

        // Load jails from database if configured
        if let Some(store) = self.store.clone() {
            self.load_jails_from_db(&store)?;
        }

        // Mark as running
        self.running = true;
        Ok(())
    }

    /// Load jails from database and sync JIDs with FreeBSD kernel
    fn load_jails_from_db(&mut self, store: &JailStore) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading jails from database: {:?}", store.db_path());

        let jail_rows = store.get_all_jails()?;
        let mut loaded_count = 0;

        for row in jail_rows {
            let name = row.name.clone();
            match Jail::from_db_row(row) {
                Ok(mut jail) => {
                    // Reset JID to -1 before syncing with kernel
                    jail.set_jid(-1);

                    // Sync with FreeBSD kernel - check if jail is actually running
                    #[cfg(target_os = "freebsd")]
                    {
                        let actual_jid = self.get_jid_from_kernel(jail.name());
                        if let Some(jid) = actual_jid {
                            jail.set_jid(jid);
                            jail.set_state(JailState::Running);
                            debug!("Jail '{}' is running with JID {}", jail.name(), jid);
                        } else {
                            // Jail is not running, ensure state is Stopped or Created
                            if jail.state() == JailState::Running {
                                jail.set_state(JailState::Stopped);
                            }
                            debug!("Jail '{}' is not running", jail.name());
                        }
                    }

                    self.jails.insert(name.clone(), jail);
                    loaded_count += 1;
                }
                Err(e) => {
                    warn!("Failed to load jail '{}' from database: {}", name, e);
                }
            }
        }

        info!("Loaded {} jails from database", loaded_count);
        Ok(())
    }

    /// Query FreeBSD kernel for JID by jail name
    #[cfg(target_os = "freebsd")]
    fn get_jid_from_kernel(&self, name: &str) -> Option<i32> {
        use std::ffi::CString;
        use std::mem;

        let name_param = CString::new("name").unwrap();
        let name_value = CString::new(name).ok()?;
        let jid_param = CString::new("jid").unwrap();

        let mut jid_out: libc::c_int = 0;

        let iovs = [
            libc::iovec {
                iov_base: name_param.as_ptr() as *mut libc::c_void,
                iov_len: name_param.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: name_value.as_ptr() as *mut libc::c_void,
                iov_len: name_value.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: jid_param.as_ptr() as *mut libc::c_void,
                iov_len: jid_param.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: &mut jid_out as *mut libc::c_int as *mut libc::c_void,
                iov_len: mem::size_of::<libc::c_int>(),
            },
        ];

        let result = unsafe {
            libc::jail_get(iovs.as_ptr() as *mut libc::iovec, iovs.len() as libc::c_uint, 0)
        };

        if result > 0 {
            Some(result)
        } else {
            None
        }
    }

    /// Stop the jail manager service
    pub async fn stop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.running {
            return Err("JailManager is not running".into());
        }

        // Stop all running jails
        for jail in self.jails.values_mut() {
            if jail.is_running() {
                let _ = jail.stop();
            }
        }

        self.running = false;
        Ok(())
    }

    /// Check if the manager is running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Add a jail to the manager
    pub fn add_jail(&mut self, name: &str) -> Result<(), JailError> {
        if self.jails.contains_key(name) {
            return Err(JailError::CreationFailed(format!(
                "Jail '{}' already exists",
                name
            )));
        }

        let jail = Jail::create(name)?;

        // Persist to database if configured
        if let Some(ref store) = self.store {
            let row = jail.to_db_row();
            if let Err(e) = store.insert_jail(&row) {
                error!("Failed to persist jail '{}' to database: {}", name, e);
                // Continue anyway - persistence failures shouldn't block jail operations
            }
        }

        self.jails.insert(name.to_string(), jail);
        Ok(())
    }

    /// Get a jail by name
    pub fn get_jail(&self, name: &str) -> Option<&Jail> {
        self.jails.get(name)
    }

    /// Get a mutable reference to a jail by name
    pub fn get_jail_mut(&mut self, name: &str) -> Option<&mut Jail> {
        self.jails.get_mut(name)
    }

    /// Start a jail by name
    pub fn start_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .get_mut(name)
            .ok_or_else(|| JailError::StartFailed(format!("Jail '{}' not found", name)))?;

        jail.start()?;

        // Persist state change to database if configured
        if let Some(ref store) = self.store {
            let row = jail.to_db_row();
            if let Err(e) = store.update_jail(&row) {
                error!("Failed to persist jail '{}' state to database: {}", name, e);
            }
        }

        Ok(())
    }

    /// Stop a jail by name
    pub fn stop_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .get_mut(name)
            .ok_or_else(|| JailError::StopFailed(format!("Jail '{}' not found", name)))?;

        jail.stop()?;

        // Persist state change to database if configured
        if let Some(ref store) = self.store {
            let row = jail.to_db_row();
            if let Err(e) = store.update_jail(&row) {
                error!("Failed to persist jail '{}' state to database: {}", name, e);
            }
        }

        Ok(())
    }

    /// Remove a jail by name
    pub fn remove_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .remove(name)
            .ok_or_else(|| JailError::DestroyFailed(format!("Jail '{}' not found", name)))?;

        jail.destroy()?;

        // Remove from database if configured
        if let Some(ref store) = self.store {
            if let Err(e) = store.delete_jail(name) {
                error!("Failed to delete jail '{}' from database: {}", name, e);
            }
        }

        Ok(())
    }

    /// Get all jail names
    pub fn jail_names(&self) -> Vec<String> {
        self.jails.keys().cloned().collect()
    }

    /// Get the number of jails
    pub fn jail_count(&self) -> usize {
        self.jails.len()
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Register a bootstrap progress tracker for a jail
    pub async fn register_bootstrap_tracker(&mut self, name: String, sender: BootstrapProgressSender) {
        // Store the sender for later use
        self.bootstrap_tracker.insert(name.clone(), sender);

        // Initialize progress with defaults
        self.bootstrap_progress.insert(name, BootstrapProgress {
            status: BootstrapStatus::Initializing,
            progress: 0,
            current_step: "Bootstrap starting...".to_string(),
            version: "unknown".to_string(),
            architecture: "unknown".to_string(),
        });
    }

    /// Get the current bootstrap progress for a jail
    pub async fn get_bootstrap_progress(&self, name: &str) -> Option<BootstrapProgress> {
        self.bootstrap_progress.get(name).cloned()
    }

    /// Send a bootstrap progress update
    pub async fn send_bootstrap_progress(&mut self, name: &str, status: BootstrapStatus) -> Result<(), mpsc::error::SendError<BootstrapProgress>> {
        if let Some(progress) = self.bootstrap_progress.get_mut(name) {
            progress.status = status.clone();
            match &status {
                BootstrapStatus::Complete => progress.progress = 100,
                BootstrapStatus::Failed(_) => progress.progress = 0,
                _ => {}
            }
            progress.current_step = match &status {
                BootstrapStatus::Complete => "Bootstrap completed".to_string(),
                BootstrapStatus::Failed(msg) => format!("Bootstrap failed: {}", msg),
                _ => "In progress".to_string(),
            };

            // Send update through the channel
            if let Some(sender) = self.bootstrap_tracker.get(name) {
                let _ = sender.try_send(progress.clone());
            }

            Ok(())
        } else {
            Err(mpsc::error::SendError(BootstrapProgress {
                status,
                progress: 0,
                current_step: "".to_string(),
                version: "".to_string(),
                architecture: "".to_string(),
            }))
        }
    }

    /// Remove bootstrap tracker for a jail
    pub async fn remove_bootstrap_tracker(&mut self, name: &str) {
        self.bootstrap_tracker.remove(name);
        self.bootstrap_progress.remove(name);
    }
}

impl Default for JailManager {
    fn default() -> Self {
        Self::with_default_socket()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manager_create() {
        let manager = JailManager::new("/tmp/test.sock");
        assert!(!manager.is_running());
        assert_eq!(manager.socket_path().to_str().unwrap(), "/tmp/test.sock");
        assert_eq!(manager.jail_count(), 0);
    }

    #[tokio::test]
    async fn test_manager_default() {
        let manager = JailManager::default();
        assert_eq!(manager.socket_path().to_str().unwrap(), "/var/run/kawakaze.sock");
    }

    #[tokio::test]
    async fn test_manager_start() {
        let mut manager = JailManager::new("/tmp/test.sock");
        assert!(!manager.is_running());

        let result = manager.start().await;
        assert!(result.is_ok());
        assert!(manager.is_running());
    }

    #[tokio::test]
    async fn test_manager_start_already_running() {
        let mut manager = JailManager::new("/tmp/test.sock");
        manager.start().await.unwrap();

        let result = manager.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already running"));
    }

    #[tokio::test]
    async fn test_manager_stop() {
        let mut manager = JailManager::new("/tmp/test.sock");
        manager.start().await.unwrap();

        let result = manager.stop().await;
        assert!(result.is_ok());
        assert!(!manager.is_running());
    }

    #[tokio::test]
    async fn test_manager_stop_not_running() {
        let mut manager = JailManager::new("/tmp/test.sock");

        let result = manager.stop().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not running"));
    }

    #[tokio::test]
    async fn test_add_jail() {
        let mut manager = JailManager::new("/tmp/test.sock");

        let result = manager.add_jail("test_jail");
        assert!(result.is_ok());
        assert_eq!(manager.jail_count(), 1);
        assert!(manager.jail_names().contains(&"test_jail".to_string()));
    }

    #[tokio::test]
    async fn test_add_duplicate_jail() {
        let mut manager = JailManager::new("/tmp/test.sock");
        manager.add_jail("test_jail").unwrap();

        let result = manager.add_jail("test_jail");
        assert!(result.is_err());

        match result {
            Err(JailError::CreationFailed(msg)) => {
                assert!(msg.contains("already exists"));
            }
            _ => panic!("Expected CreationFailed error"),
        }
    }

    #[tokio::test]
    async fn test_get_jail() {
        let mut manager = JailManager::new("/tmp/test.sock");
        manager.add_jail("test_jail").unwrap();

        let jail = manager.get_jail("test_jail");
        assert!(jail.is_some());
        assert_eq!(jail.unwrap().name(), "test_jail");
    }

    #[tokio::test]
    async fn test_get_jail_not_found() {
        let manager = JailManager::new("/tmp/test.sock");

        let jail = manager.get_jail("nonexistent");
        assert!(jail.is_none());
    }

    #[tokio::test]
    async fn test_start_jail() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let mut manager = JailManager::new("/tmp/test.sock");
        manager.add_jail("test_jail").unwrap();

        let result = manager.start_jail("test_jail");
        assert!(result.is_ok());

        let jail = manager.get_jail("test_jail").unwrap();
        assert!(jail.is_running());
    }

    #[tokio::test]
    async fn test_start_jail_not_found() {
        let mut manager = JailManager::new("/tmp/test.sock");

        let result = manager.start_jail("nonexistent");
        assert!(result.is_err());

        match result {
            Err(JailError::StartFailed(msg)) => {
                assert!(msg.contains("not found"));
            }
            _ => panic!("Expected StartFailed error"),
        }
    }

    #[tokio::test]
    async fn test_stop_jail() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let mut manager = JailManager::new("/tmp/test.sock");
        manager.add_jail("test_jail").unwrap();
        manager.start_jail("test_jail").unwrap();

        let result = manager.stop_jail("test_jail");
        assert!(result.is_ok());

        let jail = manager.get_jail("test_jail").unwrap();
        assert!(!jail.is_running());
    }

    #[tokio::test]
    async fn test_stop_jail_not_found() {
        let mut manager = JailManager::new("/tmp/test.sock");

        let result = manager.stop_jail("nonexistent");
        assert!(result.is_err());

        match result {
            Err(JailError::StopFailed(msg)) => {
                assert!(msg.contains("not found"));
            }
            _ => panic!("Expected StopFailed error"),
        }
    }

    #[tokio::test]
    async fn test_remove_jail() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let mut manager = JailManager::new("/tmp/test.sock");
        manager.add_jail("test_jail").unwrap();
        manager.start_jail("test_jail").unwrap();

        let result = manager.remove_jail("test_jail");
        assert!(result.is_ok());
        assert_eq!(manager.jail_count(), 0);
    }

    #[tokio::test]
    async fn test_remove_jail_not_found() {
        let mut manager = JailManager::new("/tmp/test.sock");

        let result = manager.remove_jail("nonexistent");
        assert!(result.is_err());

        match result {
            Err(JailError::DestroyFailed(msg)) => {
                assert!(msg.contains("not found"));
            }
            _ => panic!("Expected DestroyFailed error"),
        }
    }

    #[tokio::test]
    async fn test_multiple_jails() {
        let mut manager = JailManager::new("/tmp/test.sock");

        manager.add_jail("jail1").unwrap();
        manager.add_jail("jail2").unwrap();
        manager.add_jail("jail3").unwrap();

        assert_eq!(manager.jail_count(), 3);

        let names = manager.jail_names();
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"jail1".to_string()));
        assert!(names.contains(&"jail2".to_string()));
        assert!(names.contains(&"jail3".to_string()));
    }

    #[tokio::test]
    async fn test_manager_stops_jails_on_shutdown() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let mut manager = JailManager::new("/tmp/test.sock");

        manager.add_jail("jail1").unwrap();
        manager.add_jail("jail2").unwrap();

        manager.start_jail("jail1").unwrap();
        manager.start_jail("jail2").unwrap();

        assert!(manager.get_jail("jail1").unwrap().is_running());
        assert!(manager.get_jail("jail2").unwrap().is_running());

        // Start the manager, then stop it to verify jails are stopped
        manager.start().await.unwrap();
        manager.stop().await.unwrap();

        assert!(!manager.get_jail("jail1").unwrap().is_running());
        assert!(!manager.get_jail("jail2").unwrap().is_running());
    }

    #[tokio::test]
    async fn test_jail_names() {
        let mut manager = JailManager::new("/tmp/test.sock");

        manager.add_jail("alpha").unwrap();
        manager.add_jail("beta").unwrap();

        let mut names = manager.jail_names();
        names.sort(); // Order is not guaranteed

        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "alpha");
        assert_eq!(names[1], "beta");
    }
}
