//! Jail management module
//!
//! Interfaces with FreeBSD's jail system using libc.

use std::ffi::{CString, NulError};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Represents a FreeBSD jail
pub struct Jail {
    name: String,
    jid: i32,
    state: JailState,
    path: Option<String>,
    ip: Option<String>,
    vnet_interface: Option<String>,
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
            vnet_interface: None,
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

    /// Set the VNET interface (e.g., "epair0b")
    /// This interface will be automatically moved into the jail during creation
    pub fn with_vnet_interface(mut self, interface: &str) -> Result<Self, JailError> {
        self.vnet_interface = Some(interface.to_string());
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
            // Get the jail path for devfs mounting
            let jail_path = self.path.clone().unwrap_or_else(|| format!("/tmp/{}", self.name));

            // VNET is enabled when an IP is allocated
            let vnet = self.ip.is_some();

            self.jid = create_freebsd_jail(
                &self.name,
                self.path.as_deref(),
                self.ip.as_deref(),
                self.vnet_interface.as_deref(),
                vnet
            )?;

            // Mount devfs inside the jail for device access (needed by commands like top)
            mount_devfs(&self.name, &jail_path)?;

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
            // Get the jail path for devfs unmounting
            let jail_path = self.path.clone().unwrap_or_else(|| format!("/tmp/{}", self.name));

            // Unmount devfs before removing the jail
            let _ = unmount_devfs(&jail_path); // Ignore errors, devfs might not be mounted

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

    /// Execute a command inside the jail
    ///
    /// This runs the specified command with arguments inside the running jail using jexec.
    /// The PATH environment variable is set to ensure commands work correctly.
    pub fn exec(&self, command: &str, args: &[String]) -> Result<(), JailError> {
        if self.state != JailState::Running {
            return Err(JailError::StartFailed(format!(
                "Jail '{}' is not running", self.name
            )));
        }

        #[cfg(target_os = "freebsd")]
        {
            // Build the jexec command
            // jexec <jail_name> env PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin:~/bin <command> <args...>
            let mut cmd = Command::new("jexec");
            cmd.arg(&self.name);

            // Set PATH environment variable for command execution
            cmd.env("PATH", "/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin:~/bin");

            cmd.arg(command);
            cmd.args(args);

            // Execute the command and wait for it to complete
            let output = cmd.output()
                .map_err(|e| JailError::StartFailed(format!(
                    "Failed to execute jexec: {}", e
                )))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                return Err(JailError::StartFailed(format!(
                    "Command failed in jail '{}': {}{}",
                    self.name,
                    stdout,
                    if !stderr.is_empty() { format!("\n{}", stderr) } else { String::new() }
                )));
            }

            Ok(())
        }

        #[cfg(not(target_os = "freebsd"))]
        {
            Err(JailError::StartFailed(
                "jexec is only supported on FreeBSD".into()
            ))
        }
    }
}

impl JailState {
    /// Convert JailState to/from string for database storage
    pub fn as_str(&self) -> &'static str {
        match self {
            JailState::Created => "created",
            JailState::Running => "running",
            JailState::Stopped => "stopped",
        }
    }

    pub fn from_str(s: &str) -> Result<Self, JailError> {
        match s {
            "created" => Ok(JailState::Created),
            "running" => Ok(JailState::Running),
            "stopped" => Ok(JailState::Stopped),
            _ => Err(JailError::InvalidState(format!("Invalid jail state: {}", s))),
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

impl Jail {
    /// Convert Jail to database row for persistence
    pub fn to_db_row(&self) -> crate::store::JailRow {
        crate::store::JailRow {
            name: self.name.clone(),
            path: self.path.clone(),
            ip: self.ip.clone(),
            state: self.state.as_str().to_string(),
            jid: self.jid,
        }
    }

    /// Create Jail from database row
    pub fn from_db_row(row: crate::store::JailRow) -> Result<Self, JailError> {
        let state = JailState::from_str(&row.state)?;

        Ok(Self {
            name: row.name,
            jid: row.jid,
            state,
            path: row.path,
            ip: row.ip,
            vnet_interface: None,
        })
    }

    /// Set the JID (used when syncing with kernel)
    pub(crate) fn set_jid(&mut self, jid: i32) {
        self.jid = jid;
    }

    /// Set the state (used when syncing with kernel)
    pub(crate) fn set_state(&mut self, state: JailState) {
        self.state = state;
    }
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

    /// Create a FreeBSD jail using jail_set system call or jail command
    ///
    /// For VNET jails, we use the `jail` command because jail_set() requires
    /// JAIL_ATTACH when creating VNET jails, which would attach the backend
    /// process to the jail.
    ///
    /// For non-VNET jails, we use jail_set() directly for better control.
    pub fn create_freebsd_jail(
        name: &str,
        path: Option<&str>,
        ip: Option<&str>,
        vnet_interface: Option<&str>,
        vnet: bool,
    ) -> Result<i32, JailError> {
        // For VNET jails, use the jail command instead of jail_set()
        if vnet {
            return create_freebsd_jail_with_command(name, path, ip, vnet_interface);
        }

        // For non-VNET jails, use jail_set() system call
        create_freebsd_jail_with_syscall(name, path, ip)
    }

    /// Create a VNET jail using the jail command
    fn create_freebsd_jail_with_command(
        name: &str,
        path: Option<&str>,
        ip: Option<&str>,
        vnet_interface: Option<&str>,
    ) -> Result<i32, JailError> {
        use std::process::Command;

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

        // Build the jail command
        // jail -c name=<name> path=<path> host.hostname=<name> persist vnet [vnet.interface=<iface>]
        // Note: For VNET jails, we do NOT pass ip4.addr to the jail command.
        // The IP will be configured on the epair interface by the networking module.
        let mut cmd = Command::new("jail");
        cmd.arg("-c");
        cmd.arg(format!("name={}", name));
        cmd.arg(format!("path={}", jail_path));
        cmd.arg(format!("host.hostname={}", name));
        cmd.arg("persist");
        cmd.arg("vnet");

        // If vnet_interface is specified, add it as a parameter
        // This is the KEY FIX: using vnet.interface=<epairXb> during jail creation
        // moves the epair into the jail DURING creation, which works reliably
        if let Some(iface) = vnet_interface {
            cmd.arg(format!("vnet.interface={}", iface));
        }

        tracing::debug!("Creating VNET jail with command: {:?}", cmd);

        // Execute the jail command
        let output = cmd.output()
            .map_err(|e| JailError::CreationFailed(format!(
                "Failed to execute jail command: {}", e
            )))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(JailError::CreationFailed(format!(
                "jail command failed: {}", stderr
            )));
        }

        // Get the JID by name
        let jid = get_jid_by_name(name)?;

        tracing::debug!("Created VNET jail '{}' with JID: {}", name, jid);
        Ok(jid)
    }

    /// Get the JID of a jail by its name using jail_get system call
    fn get_jid_by_name(name: &str) -> Result<i32, JailError> {
        let name_cstring = CString::new(name)?;
        let name_param = CString::new("name").unwrap();

        let iovs = [
            libc::iovec {
                iov_base: name_param.as_ptr() as *mut libc::c_void,
                iov_len: name_param.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: name_cstring.as_ptr() as *mut libc::c_void,
                iov_len: name.len() + 1,
            },
        ];

        let jid = unsafe {
            libc::jail_get(iovs.as_ptr() as *mut libc::iovec, iovs.len() as libc::c_uint, 0)
        };

        if jid < 0 {
            return Err(JailError::CreationFailed(format!(
                "Failed to get JID for jail '{}': {}",
                name,
                std::io::Error::last_os_error()
            )));
        }

        Ok(jid)
    }

    /// Create a non-VNET jail using jail_set system call
    fn create_freebsd_jail_with_syscall(
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

        // Add metadata marking for FreeBSD 15.0+ (meta.managed_by=kawakaze)
        // This allows external tools to identify Kawakaze-managed jails
        // Note: On older FreeBSD versions without metadata support, jail_set
        // will return EINVAL. We handle this gracefully by trying without metadata.
        let meta_key = CString::new("meta.managed_by").unwrap();
        let meta_value = CString::new("kawakaze").unwrap();

        iovs.push(libc::iovec {
            iov_base: meta_key.as_ptr() as *mut libc::c_void,
            iov_len: meta_key.as_bytes().len() + 1,
        });
        iovs.push(libc::iovec {
            iov_base: meta_value.as_ptr() as *mut libc::c_void,
            iov_len: meta_value.as_bytes().len() + 1,
        });

        // Call jail_set function from libc
        // Use JAIL_CREATE to create the jail, but don't attach (JAIL_ATTACH)
        // so we can continue managing it from outside the jail
        let flags = libc::JAIL_CREATE;

        let jid = unsafe {
            libc::jail_set(iovs.as_mut_ptr(), iovs.len() as libc::c_uint, flags)
        };

        if jid < 0 {
            let err = std::io::Error::last_os_error();
            // On older FreeBSD versions, metadata may not be supported (EINVAL)
            // Retry without the metadata parameter
            if err.raw_os_error() == Some(libc::EINVAL) {
                tracing::debug!("Metadata parameter not supported, retrying without metadata");
                // Remove the last two iovecs (metadata)
                iovs.pop();
                iovs.pop();

                let jid = unsafe {
                    libc::jail_set(iovs.as_mut_ptr(), iovs.len() as libc::c_uint, flags)
                };

                if jid < 0 {
                    return Err(JailError::CreationFailed(format!(
                        "jail_set failed: {}",
                        std::io::Error::last_os_error()
                    )));
                }

                return Ok(jid);
            }

            return Err(JailError::CreationFailed(format!(
                "jail_set failed: {}",
                err
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

    /// Mount devfs inside a jail
    ///
    /// This mounts the devfs filesystem at /dev inside the jail path,
    /// which is necessary for commands like `top` to access device nodes.
    pub fn mount_devfs(jail_name: &str, jail_path: &str) -> Result<(), JailError> {
        // Create /dev directory if it doesn't exist
        let dev_path = format!("{}/dev", jail_path);
        if let Err(e) = fs::create_dir_all(&dev_path) {
            return Err(JailError::CreationFailed(format!(
                "Failed to create /dev directory in jail '{}': {}",
                jail_name, e
            )));
        }

        // Mount devfs using the mount command
        // On FreeBSD, the correct syntax is: mount -t devfs devfs /path
        // We can also use -o to specify options like ruleset
        let output = Command::new("mount")
            .arg("-t")
            .arg("devfs")
            .arg("devfs")
            .arg(&dev_path)
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(JailError::CreationFailed(format!(
                    "Failed to mount devfs in jail '{}': {}",
                    jail_name, stderr
                )))
            }
            Err(e) => {
                Err(JailError::CreationFailed(format!(
                    "Failed to execute mount command for jail '{}': {}",
                    jail_name, e
                )))
            }
        }
    }

    /// Unmount devfs from a jail
    ///
    /// This unmounts the devfs filesystem from the jail path.
    /// Silently succeeds if devfs is not mounted.
    pub fn unmount_devfs(jail_path: &str) -> Result<(), JailError> {
        let dev_path = format!("{}/dev", jail_path);

        // Unmount devfs using the umount command with -f to force unmount
        let output = Command::new("umount")
            .arg("-f")
            .arg(&dev_path)
            .output();

        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                // Check if the error is because it's not mounted
                // In that case, we silently succeed
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not mounted") || stderr.contains("not found") {
                    Ok(())
                } else {
                    tracing::debug!("Failed to unmount devfs from '{}': {}", dev_path, stderr);
                    // Don't fail if unmount fails - it might already be unmounted
                    Ok(())
                }
            }
            Err(e) => {
                tracing::debug!("Failed to execute umount command for '{}': {}", dev_path, e);
                // Don't fail - it might already be unmounted
                Ok(())
            }
        }
    }
}

#[cfg(target_os = "freebsd")]
use freebsd::{create_freebsd_jail, remove_freebsd_jail, check_jail_exists, mount_devfs, unmount_devfs};

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

    #[test]
    fn test_jail_exec_nonexistent_jail() {
        let jail = Jail::create("test_exec_nonexistent").unwrap();

        // Try to exec in a jail that isn't running
        let result = jail.exec("echo", &["hello".to_string()]);
        assert!(result.is_err());

        match result {
            Err(JailError::StartFailed(msg)) => {
                assert!(msg.contains("not running") || msg.contains("failed"));
            }
            _ => panic!("Expected StartFailed error"),
        }
    }

    #[test]
    fn test_jail_exec_empty_command() {
        let jail = Jail::create("test_exec_empty").unwrap();

        let result = jail.exec("", &[]);
        assert!(result.is_err());

        match result {
            Err(JailError::StartFailed(msg)) => {
                assert!(msg.contains("empty") || msg.contains("not running"));
            }
            _ => panic!("Expected StartFailed error"),
        }
    }

    #[test]
    fn test_jail_exec_with_args() {
        let jail = Jail::create("test_exec_args").unwrap();

        // Just test that the method accepts args properly
        // Actual execution will fail since jail isn't running
        let result = jail.exec("echo", &["hello".to_string(), "world".to_string()]);
        assert!(result.is_err());
    }
}
