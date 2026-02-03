//! Jail management module
//!
//! Interfaces with FreeBSD's jail system using libc.

/// Represents a FreeBSD jail
pub struct Jail {
    name: String,
    jid: i32,
    state: JailState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JailState {
    Created,
    Running,
    Stopped,
}

impl Jail {
    /// Create a new jail configuration (does not actually create the FreeBSD jail)
    pub fn create(name: &str) -> Result<Self, JailError> {
        if name.is_empty() {
            return Err(JailError::CreationFailed("Jail name cannot be empty".into()));
        }

        // Validate jail name (alphanumeric and underscores only)
        if !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(JailError::CreationFailed(format!(
                "Invalid jail name '{}': only alphanumeric, underscore, and hyphen characters allowed",
                name
            )));
        }

        Ok(Self {
            name: name.to_string(),
            jid: -1,
            state: JailState::Created,
        })
    }

    /// Get the jail name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the jail ID (JID)
    pub fn jid(&self) -> i32 {
        self.jid
    }

    /// Get the current state of the jail
    pub fn state(&self) -> JailState {
        self.state
    }

    /// Check if the jail is running
    pub fn is_running(&self) -> bool {
        self.state == JailState::Running
    }

    /// Start the jail
    ///
    /// This creates and starts a FreeBSD jail with the given configuration.
    /// Note: This is a stub implementation that simulates jail creation.
    /// Actual FreeBSD jail creation would use jail(2) system call.
    pub fn start(&mut self) -> Result<(), JailError> {
        if self.state == JailState::Running {
            return Err(JailError::StartFailed(format!(
                "Jail '{}' is already running",
                self.name
            )));
        }

        // In a real implementation, this would:
        // 1. Create a jail parameter structure with jail_set(2)
        // 2. Call jail(2) to create the jail
        // 3. Store the returned JID

        // For now, we simulate successful jail creation
        self.jid = simulate_jail_create(&self.name)?;
        self.state = JailState::Running;

        Ok(())
    }

    /// Stop the jail
    ///
    /// This stops a running FreeBSD jail.
    /// Note: This is a stub implementation.
    pub fn stop(&mut self) -> Result<(), JailError> {
        if self.state != JailState::Running {
            return Err(JailError::StopFailed(format!(
                "Jail '{}' is not running",
                self.name
            )));
        }

        // In a real implementation, this would call jail_remove(self.jid)

        simulate_jail_remove(self.jid)?;
        self.jid = -1;
        self.state = JailState::Stopped;

        Ok(())
    }

    /// Destroy the jail
    ///
    /// This destroys the jail and cleans up resources.
    /// Note: This is a stub implementation.
    pub fn destroy(mut self) -> Result<(), JailError> {
        // Stop the jail first if it's running
        if self.state == JailState::Running {
            self.stop()?;
        }

        // In a real implementation, this would clean up any jail configuration

        Ok(())
    }

    /// Get jail information
    pub fn info(&self) -> JailInfo {
        JailInfo {
            name: self.name.clone(),
            jid: self.jid,
            state: self.state,
        }
    }
}

/// Jail information
#[derive(Debug, Clone)]
pub struct JailInfo {
    pub name: String,
    pub jid: i32,
    pub state: JailState,
}

/// Jail operation errors
#[derive(Debug)]
pub enum JailError {
    CreationFailed(String),
    StartFailed(String),
    StopFailed(String),
    DestroyFailed(String),
    InvalidState(String),
}

impl std::fmt::Display for JailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JailError::CreationFailed(msg) => write!(f, "Failed to create jail: {}", msg),
            JailError::StartFailed(msg) => write!(f, "Failed to start jail: {}", msg),
            JailError::StopFailed(msg) => write!(f, "Failed to stop jail: {}", msg),
            JailError::DestroyFailed(msg) => write!(f, "Failed to destroy jail: {}", msg),
            JailError::InvalidState(msg) => write!(f, "Invalid jail state: {}", msg),
        }
    }
}

impl std::error::Error for JailError {}

/// Simulate jail creation for testing purposes
///
/// In production, this would use the FreeBSD jail(2) system call.
/// Returns a simulated jail ID.
fn simulate_jail_create(name: &str) -> Result<i32, JailError> {
    use std::sync::atomic::{AtomicI32, Ordering};

    static NEXT_JID: AtomicI32 = AtomicI32::new(1);

    // Simulate potential failure conditions
    if name.contains("fail") {
        return Err(JailError::CreationFailed(format!(
            "Simulated failure creating jail '{}'",
            name
        )));
    }

    Ok(NEXT_JID.fetch_add(1, Ordering::SeqCst))
}

/// Simulate jail removal for testing purposes
///
/// In production, this would use the FreeBSD jail_remove(2) system call.
fn simulate_jail_remove(jid: i32) -> Result<(), JailError> {
    // Simulate potential failure conditions
    if jid < 0 {
        return Err(JailError::StopFailed(format!(
            "Invalid jail ID: {}",
            jid
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jail_create() {
        let jail = Jail::create("test_jail");
        assert!(jail.is_ok());

        let jail = jail.unwrap();
        assert_eq!(jail.name(), "test_jail");
        assert_eq!(jail.state(), JailState::Created);
        assert!(!jail.is_running());
    }

    #[test]
    fn test_jail_create_empty_name() {
        let jail = Jail::create("");
        assert!(jail.is_err());

        match jail {
            Err(JailError::CreationFailed(msg)) => {
                assert!(msg.contains("cannot be empty"));
            }
            _ => panic!("Expected CreationFailed error"),
        }
    }

    #[test]
    fn test_jail_create_invalid_name() {
        let jail = Jail::create("invalid name!");
        assert!(jail.is_err());

        match jail {
            Err(JailError::CreationFailed(msg)) => {
                assert!(msg.contains("Invalid jail name"));
            }
            _ => panic!("Expected CreationFailed error"),
        }
    }

    #[test]
    fn test_jail_start() {
        let mut jail = Jail::create("test_start").unwrap();
        assert!(!jail.is_running());

        let result = jail.start();
        assert!(result.is_ok());
        assert!(jail.is_running());
        assert_eq!(jail.state(), JailState::Running);
        assert!(jail.jid() >= 1); // JID should be >= 1
    }

    #[test]
    fn test_jail_start_already_running() {
        let mut jail = Jail::create("test_double_start").unwrap();
        jail.start().unwrap();

        let result = jail.start();
        assert!(result.is_err());

        match result {
            Err(JailError::StartFailed(msg)) => {
                assert!(msg.contains("already running"));
            }
            _ => panic!("Expected StartFailed error"),
        }
    }

    #[test]
    fn test_jail_stop() {
        let mut jail = Jail::create("test_stop").unwrap();
        jail.start().unwrap();

        let result = jail.stop();
        assert!(result.is_ok());
        assert_eq!(jail.state(), JailState::Stopped);
        assert!(!jail.is_running());
        assert_eq!(jail.jid(), -1);
    }

    #[test]
    fn test_jail_stop_not_running() {
        let mut jail = Jail::create("test_stop_not_running").unwrap();

        let result = jail.stop();
        assert!(result.is_err());

        match result {
            Err(JailError::StopFailed(msg)) => {
                assert!(msg.contains("not running"));
            }
            _ => panic!("Expected StopFailed error"),
        }
    }

    #[test]
    fn test_jail_destroy() {
        let mut jail = Jail::create("test_destroy").unwrap();
        jail.start().unwrap();

        let result = jail.destroy();
        assert!(result.is_ok());
    }

    #[test]
    fn test_jail_destroy_stopped() {
        let jail = Jail::create("test_destroy_stopped").unwrap();

        let result = jail.destroy();
        assert!(result.is_ok());
    }

    #[test]
    fn test_jail_info() {
        let jail = Jail::create("test_info").unwrap();
        let info = jail.info();

        assert_eq!(info.name, "test_info");
        assert_eq!(info.state, JailState::Created);
        assert_eq!(info.jid, -1);
    }

    #[test]
    fn test_jail_info_running() {
        let mut jail = Jail::create("test_info_running").unwrap();
        jail.start().unwrap();

        let info = jail.info();

        assert_eq!(info.name, "test_info_running");
        assert_eq!(info.state, JailState::Running);
        assert!(info.jid >= 1);
    }

    #[test]
    fn test_multiple_jails() {
        let mut jail1 = Jail::create("jail1").unwrap();
        let mut jail2 = Jail::create("jail2").unwrap();

        jail1.start().unwrap();
        jail2.start().unwrap();

        // JIDs should be different
        assert_ne!(jail1.jid(), jail2.jid());

        jail1.stop().unwrap();
        jail2.stop().unwrap();
    }

    #[test]
    fn test_jail_lifecycle() {
        let mut jail = Jail::create("lifecycle_test").unwrap();

        // Create
        assert_eq!(jail.state(), JailState::Created);

        // Start
        jail.start().unwrap();
        assert_eq!(jail.state(), JailState::Running);

        // Stop
        jail.stop().unwrap();
        assert_eq!(jail.state(), JailState::Stopped);

        // Destroy
        jail.destroy().unwrap();
    }

    #[test]
    fn test_jail_name_with_hyphen() {
        let jail = Jail::create("test-jail-123");
        assert!(jail.is_ok());

        let jail = jail.unwrap();
        assert_eq!(jail.name(), "test-jail-123");
    }

    #[test]
    fn test_jail_name_with_underscore() {
        let jail = Jail::create("test_jail_456");
        assert!(jail.is_ok());

        let jail = jail.unwrap();
        assert_eq!(jail.name(), "test_jail_456");
    }
}
