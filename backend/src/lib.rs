//! Kawakaze backend - Jail management service
//!
//! This module handles the actual management of FreeBSD jails,
//! communicating with clients through a unix socket.

pub mod jail;

use crate::jail::{Jail, JailError};
use std::collections::HashMap;
use std::path::PathBuf;

/// Jail manager - handles jail lifecycle
pub struct JailManager {
    socket_path: PathBuf,
    jails: HashMap<String, Jail>,
    running: bool,
}

impl JailManager {
    /// Create a new jail manager
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            jails: HashMap::new(),
            running: false,
        }
    }

    /// Create a jail manager with default socket path
    pub fn with_default_socket() -> Self {
        Self::new("/var/run/kawakaze.sock")
    }

    /// Start the jail manager service
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.running {
            return Err("JailManager is already running".into());
        }

        // TODO: Start unix socket listener
        // For now, we just mark as running
        self.running = true;
        Ok(())
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

        jail.start()
    }

    /// Stop a jail by name
    pub fn stop_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .get_mut(name)
            .ok_or_else(|| JailError::StopFailed(format!("Jail '{}' not found", name)))?;

        jail.stop()
    }

    /// Remove a jail by name
    pub fn remove_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .remove(name)
            .ok_or_else(|| JailError::DestroyFailed(format!("Jail '{}' not found", name)))?;

        jail.destroy()
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
