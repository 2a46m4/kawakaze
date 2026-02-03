//! Jail management module
//!
//! Interfaces with FreeBSD's jail system using libc.

use std::ffi::{CString, NulError};
use std::fs;
use std::path::Path;

/// Represents a FreeBSD jail
pub struct Jail {
    name: String,
    jid: i32,
    state: JailState,
    path: Option<String>,
    ip: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JailState {
    Created,
    Running,
    Stopped,
}

impl Jail {
    /// Create a new jail configuration
    pub fn create(name: &str) -> Result<Self, JailError> {
        if name.is_empty() {
            return Err(JailError::CreationFailed("Jail name cannot be empty".into()));
        }

        // Validate jail name (alphanumeric, underscore, and hyphen only)
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
            path: None,
            ip: None,
        })
    }

    /// Set the jail path (root directory)
    pub fn with_path(mut self, path: impl AsRef<Path>) -> Result<Self, JailError> {
        let path_str = path.as_ref().to_string_lossy().to_string();
        self.path = Some(path_str);
        Ok(self)
    }

    /// Set the jail IP address
    pub fn with_ip(mut self, ip: &str) -> Result<Self, JailError> {
        self.ip = Some(ip.to_string());
        Ok(self)
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
    pub fn start(&mut self) -> Result<(), JailError> {
        if self.state == JailState::Running {
            return Err(JailError::StartFailed(format!(
                "Jail '{}' is already running",
                self.name
            )));
        }

        // Check if running as root (required for jail creation)
        if !is_root() {
            return Err(JailError::StartFailed(
                "Jail creation requires root privileges".into()
            ));
        }

        #[cfg(target_os = "freebsd")]
        {
            self.jid = create_freebsd_jail(&self.name, self.path.as_deref(), self.ip.as_deref())?;
            self.state = JailState::Running;
            return Ok(());
        }

        #[cfg(not(target_os = "freebsd"))]
        {
            return Err(JailError::StartFailed(
                "Jail creation is only supported on FreeBSD".into()
            ));
        }
    }

    /// Stop the jail
    ///
    /// This stops a running FreeBSD jail.
    pub fn stop(&mut self) -> Result<(), JailError> {
        if self.state != JailState::Running {
            return Err(JailError::StopFailed(format!(
                "Jail '{}' is not running",
                self.name
            )));
        }

        if !is_root() {
            return Err(JailError::StopFailed(
                "Jail removal requires root privileges".into()
            ));
        }

        #[cfg(target_os = "freebsd")]
        {
            remove_freebsd_jail(self.jid)?;
            self.jid = -1;
            self.state = JailState::Stopped;
            return Ok(());
        }

        #[cfg(not(target_os = "freebsd"))]
        {
            return Err(JailError::StopFailed(
                "Jail removal is only supported on FreeBSD".into()
            ));
        }
    }

    /// Destroy the jail
    ///
    /// This destroys the jail and cleans up resources.
    pub fn destroy(mut self) -> Result<(), JailError> {
        // Stop the jail first if it's running
        if self.state == JailState::Running {
            self.stop()?;
        }

        // Clean up jail path if it was created by us
        if let Some(ref path) = self.path {
            // Optionally clean up the jail directory
            // For safety, we don't auto-delete paths
            let _ = path;
        }

        Ok(())
    }

    /// Get jail information
    pub fn info(&self) -> JailInfo {
        JailInfo {
            name: self.name.clone(),
            jid: self.jid,
            state: self.state,
            path: self.path.clone(),
        }
    }

    /// Check if a jail with the given JID exists
    pub fn exists(jid: i32) -> bool {
        #[cfg(target_os = "freebsd")]
        {
            check_jail_exists(jid)
        }

        #[cfg(not(target_os = "freebsd"))]
        {
            false
        }
    }
}

/// Jail information
#[derive(Debug, Clone)]
pub struct JailInfo {
    pub name: String,
    pub jid: i32,
    pub state: JailState,
    pub path: Option<String>,
}

/// Jail operation errors
#[derive(Debug)]
pub enum JailError {
    CreationFailed(String),
    StartFailed(String),
    StopFailed(String),
    DestroyFailed(String),
    InvalidState(String),
    InvalidPath(String),
}

impl std::fmt::Display for JailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JailError::CreationFailed(msg) => write!(f, "Failed to create jail: {}", msg),
            JailError::StartFailed(msg) => write!(f, "Failed to start jail: {}", msg),
            JailError::StopFailed(msg) => write!(f, "Failed to stop jail: {}", msg),
            JailError::DestroyFailed(msg) => write!(f, "Failed to destroy jail: {}", msg),
            JailError::InvalidState(msg) => write!(f, "Invalid jail state: {}", msg),
            JailError::InvalidPath(msg) => write!(f, "Invalid path: {}", msg),
        }
    }
}

impl std::error::Error for JailError {}

impl From<NulError> for JailError {
    fn from(_: NulError) -> Self {
        JailError::CreationFailed("Null byte in string".into())
    }
}

/// Check if running as root
fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::getuid() == 0 }
    }

    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(target_os = "freebsd")]
mod freebsd {
    use super::*;

    /// Create a FreeBSD jail using jail_set system call
    pub fn create_freebsd_jail(
        name: &str,
        path: Option<&str>,
        ip: Option<&str>,
    ) -> Result<i32, JailError> {
        use std::mem;

        // Determine path - use /tmp/jailname if not specified
        let default_path = format!("/tmp/{}", name);
        let jail_path = path.unwrap_or(&default_path);

        // Create jail directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(jail_path) {
            return Err(JailError::CreationFailed(format!(
                "Failed to create jail directory '{}': {}",
                jail_path, e
            )));
        }

        // Prepare jail parameters as C strings
        let name_cstring = CString::new(name)?;
        let path_cstring = CString::new(jail_path)?;
        let hostname_cstring = CString::new(name)?;
        let ip_cstring = ip.map(|p| CString::new(p)).transpose()?;

        // Create static C strings for parameter names
        let name_param = CString::new("name").unwrap();
        let path_param = CString::new("path").unwrap();
        let hostname_param = CString::new("host.hostname").unwrap();
        let persist_param = CString::new("persist").unwrap();
        let ip_param = CString::new("ip4.addr").unwrap();

        let persist_value: libc::c_int = 1;

        // Build iovec array - each parameter needs TWO iovecs: name and value
        let mut iovs: Vec<libc::iovec> = Vec::new();

        // name parameter
        iovs.push(libc::iovec {
            iov_base: name_param.as_ptr() as *mut libc::c_void,
            iov_len: name_param.as_bytes().len() + 1, // Include null terminator
        });
        iovs.push(libc::iovec {
            iov_base: name_cstring.as_ptr() as *mut libc::c_void,
            iov_len: name.len() + 1,
        });

        // path parameter
        iovs.push(libc::iovec {
            iov_base: path_param.as_ptr() as *mut libc::c_void,
            iov_len: path_param.as_bytes().len() + 1,
        });
        iovs.push(libc::iovec {
            iov_base: path_cstring.as_ptr() as *mut libc::c_void,
            iov_len: jail_path.len() + 1,
        });

        // hostname parameter
        iovs.push(libc::iovec {
            iov_base: hostname_param.as_ptr() as *mut libc::c_void,
            iov_len: hostname_param.as_bytes().len() + 1,
        });
        iovs.push(libc::iovec {
            iov_base: hostname_cstring.as_ptr() as *mut libc::c_void,
            iov_len: name.len() + 1,
        });

        // persist parameter
        iovs.push(libc::iovec {
            iov_base: persist_param.as_ptr() as *mut libc::c_void,
            iov_len: persist_param.as_bytes().len() + 1,
        });
        iovs.push(libc::iovec {
            iov_base: &persist_value as *const libc::c_int as *mut libc::c_void,
            iov_len: mem::size_of::<libc::c_int>(),
        });

        // Only add IP if specified
        if let Some(ref ip_c) = ip_cstring {
            iovs.push(libc::iovec {
                iov_base: ip_param.as_ptr() as *mut libc::c_void,
                iov_len: ip_param.as_bytes().len() + 1,
            });
            iovs.push(libc::iovec {
                iov_base: ip_c.as_ptr() as *mut libc::c_void,
                iov_len: ip_c.as_bytes().len() + 1,
            });
        }

        // Call jail_set function from libc
        // Use JAIL_CREATE to create the jail, but don't attach (JAIL_ATTACH)
        // so we can continue managing it from outside the jail
        let flags = libc::JAIL_CREATE;

        let jid = unsafe {
            libc::jail_set(iovs.as_mut_ptr(), iovs.len() as libc::c_uint, flags)
        };

        if jid < 0 {
            return Err(JailError::CreationFailed(format!(
                "jail_set failed: {}",
                std::io::Error::last_os_error()
            )));
        }

        Ok(jid)
    }

    /// Remove a FreeBSD jail
    pub fn remove_freebsd_jail(jid: i32) -> Result<(), JailError> {
        let result = unsafe { libc::jail_remove(jid) };

        if result < 0 {
            return Err(JailError::StopFailed(format!(
                "jail_remove failed: {}",
                std::io::Error::last_os_error()
            )));
        }

        Ok(())
    }

    /// Check if a jail exists
    pub fn check_jail_exists(jid: i32) -> bool {
        let _jid_out: libc::c_int = 0;

        let jid_param = CString::new("jid").unwrap();

        let iovs = [
            libc::iovec {
                iov_base: jid_param.as_ptr() as *mut libc::c_void,
                iov_len: jid_param.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: &jid as *const i32 as *mut libc::c_void,
                iov_len: std::mem::size_of::<i32>(),
            },
        ];

        let result = unsafe {
            libc::jail_get(iovs.as_ptr() as *mut libc::iovec, iovs.len() as libc::c_uint, 0)
        };

        result >= 0
    }
}

#[cfg(target_os = "freebsd")]
use freebsd::{create_freebsd_jail, remove_freebsd_jail, check_jail_exists};

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
    fn test_jail_with_path() {
        let jail = Jail::create("test_path").unwrap().with_path("/tmp/test_jail");
        assert!(jail.is_ok());

        let jail = jail.unwrap();
        assert_eq!(jail.path, Some("/tmp/test_jail".to_string()));
    }

    #[test]
    fn test_jail_with_ip() {
        let jail = Jail::create("test_ip").unwrap().with_ip("192.168.1.100");
        assert!(jail.is_ok());

        let jail = jail.unwrap();
        assert_eq!(jail.ip, Some("192.168.1.100".to_string()));
    }

    #[test]
    fn test_jail_start_not_root() {
        // This test will fail if run as root, which is fine
        if is_root() {
            return; // Skip test when running as root
        }

        let mut jail = Jail::create("test_start").unwrap();
        let result = jail.start();

        assert!(result.is_err());

        match result {
            Err(JailError::StartFailed(msg)) => {
                assert!(msg.contains("root privileges") || msg.contains("FreeBSD"));
            }
            _ => panic!("Expected StartFailed error"),
        }
    }

    #[test]
    fn test_jail_stop_not_root() {
        // This test will fail if run as root, which is fine
        if is_root() {
            return; // Skip test when running as root
        }

        let mut jail = Jail::create("test_stop").unwrap();
        jail.jid = 123; // Simulate a running jail
        jail.state = JailState::Running;

        let result = jail.stop();

        assert!(result.is_err());

        match result {
            Err(JailError::StopFailed(msg)) => {
                assert!(msg.contains("root privileges") || msg.contains("FreeBSD"));
            }
            _ => panic!("Expected StopFailed error"),
        }
    }

    #[test]
    fn test_jail_start_already_running() {
        let mut jail = Jail::create("test_double_start").unwrap();
        jail.jid = 123;
        jail.state = JailState::Running;

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
        // Simulate running jail but without root, it can't actually be stopped
        if is_root() {
            jail.jid = 123;
            jail.state = JailState::Running;
            let result = jail.destroy();
            assert!(result.is_ok());
        } else {
            // When not root, just test destroy on a stopped jail
            let jail = Jail::create("test_destroy").unwrap();
            let result = jail.destroy();
            assert!(result.is_ok());
        }
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
        jail.jid = 123;
        jail.state = JailState::Running;

        let info = jail.info();

        assert_eq!(info.name, "test_info_running");
        assert_eq!(info.state, JailState::Running);
        assert_eq!(info.jid, 123);
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

    #[test]
    fn test_jail_info_with_path() {
        let jail = Jail::create("test_info_path")
            .unwrap()
            .with_path("/tmp/custom_path")
            .unwrap();

        let info = jail.info();
        assert_eq!(info.path, Some("/tmp/custom_path".to_string()));
    }
}
