//! Networking module for Kawakaze containers
//!
//! This module handles network configuration for FreeBSD jails, including:
//! - Bridge interface management (bridge0)
//! - IP address allocation from 10.11.0.0/16
//! - epair interface creation and attachment
//! - NAT/pf configuration for internet access
//! - Port forwarding with pf rules

use std::collections::HashSet;
use std::net::IpAddr;
use std::path::Path;
use std::process::Command;
use std::fs;
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn, error};

const BRIDGE_NAME: &str = "bridge0";
const BRIDGE_IP: &str = "10.11.0.1/16";
const NETWORK_PREFIX: &str = "10.11.0";
const NETWORK_CIDR: &str = "10.11.0.0/16";
const PF_ANCHOR: &str = "kawakaze";

/// Network configuration errors
#[derive(Debug)]
pub enum NetworkError {
    BridgeCreationFailed(String),
    BridgeAlreadyExists,
    BridgeNotFound,
    EpairCreationFailed(String),
    EpairAttachmentFailed(String),
    IpAllocationFailed(String),
    IpExhausted,
    PfError(String),
    IoError(std::io::Error),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkError::BridgeCreationFailed(msg) => write!(f, "Failed to create bridge: {}", msg),
            NetworkError::BridgeAlreadyExists => write!(f, "Bridge already exists"),
            NetworkError::BridgeNotFound => write!(f, "Bridge not found"),
            NetworkError::EpairCreationFailed(msg) => write!(f, "Failed to create epair: {}", msg),
            NetworkError::EpairAttachmentFailed(msg) => write!(f, "Failed to attach epair: {}", msg),
            NetworkError::IpAllocationFailed(msg) => write!(f, "Failed to allocate IP: {}", msg),
            NetworkError::IpExhausted => write!(f, "No more IP addresses available"),
            NetworkError::PfError(msg) => write!(f, "PF error: {}", msg),
            NetworkError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for NetworkError {}

impl From<std::io::Error> for NetworkError {
    fn from(e: std::io::Error) -> Self {
        NetworkError::IoError(e)
    }
}

/// IP address allocator for containers
pub struct IpAllocator {
    allocated_ips: HashSet<std::net::Ipv4Addr>,
    next_ip: u32, // Offset from NETWORK_PREFIX
}

impl IpAllocator {
    /// Create a new IP allocator for the 10.11.0.0/16 network
    pub fn new() -> Self {
        let mut allocator = Self {
            allocated_ips: HashSet::new(),
            next_ip: 2, // Start at 10.11.0.2 (.1 is the bridge)
        };

        // Load existing allocations from state file
        if let Err(e) = allocator.load_state() {
            debug!("No existing IP allocation state found: {}", e);
        }

        allocator
    }

    /// Allocate a new IP address from the pool
    pub fn allocate(&mut self) -> Result<std::net::Ipv4Addr, NetworkError> {
        // Try to find an available IP
        for offset in self.next_ip..65534 {
            let ip = self.offset_to_ip(offset)?;

            // Skip if already allocated
            if self.allocated_ips.contains(&ip) {
                continue;
            }

            self.allocated_ips.insert(ip);
            self.next_ip = offset + 1;
            self.save_state()?;

            debug!("Allocated IP address: {}", ip);
            return Ok(ip);
        }

        // Try from the beginning in case we have gaps
        for offset in 2..self.next_ip {
            let ip = self.offset_to_ip(offset)?;

            if self.allocated_ips.contains(&ip) {
                continue;
            }

            self.allocated_ips.insert(ip);
            self.save_state()?;

            debug!("Allocated IP address from gap: {}", ip);
            return Ok(ip);
        }

        error!("IP address pool exhausted");
        Err(NetworkError::IpExhausted)
    }

    /// Allocate a specific IP address
    pub fn allocate_specific(&mut self, ip: std::net::Ipv4Addr) -> Result<(), NetworkError> {
        if !self.is_in_network(ip) {
            return Err(NetworkError::IpAllocationFailed(format!(
                "IP {} is not in the {} network", ip, NETWORK_CIDR
            )));
        }

        if self.allocated_ips.contains(&ip) {
            return Err(NetworkError::IpAllocationFailed(format!(
                "IP {} is already allocated", ip
            )));
        }

        self.allocated_ips.insert(ip);
        self.save_state()?;

        debug!("Allocated specific IP address: {}", ip);
        Ok(())
    }

    /// Release an IP address back to the pool
    pub fn release(&mut self, ip: std::net::Ipv4Addr) -> Result<(), NetworkError> {
        if self.allocated_ips.remove(&ip) {
            debug!("Released IP address: {}", ip);
            self.save_state()?;
        }

        Ok(())
    }

    /// Check if an IP is in our network
    fn is_in_network(&self, ip: std::net::Ipv4Addr) -> bool {
        let octets = ip.octets();
        octets[0] == 10 && octets[1] == 11
    }

    /// Convert offset to IP address
    fn offset_to_ip(&self, offset: u32) -> Result<std::net::Ipv4Addr, NetworkError> {
        if offset > 65534 || offset < 2 {
            return Err(NetworkError::IpAllocationFailed(format!(
                "Invalid IP offset: {}", offset
            )));
        }

        let third = (offset / 256) as u8;
        let fourth = (offset % 256) as u8;

        Ok(std::net::Ipv4Addr::new(10, 11, third, fourth))
    }

    /// Save allocation state to disk
    fn save_state(&self) -> Result<(), NetworkError> {
        let state_dir = "/var/db/kawakaze";
        fs::create_dir_all(state_dir)?;

        let state_file = Path::new(state_dir).join("ip_allocations.txt");
        let mut content = String::new();

        for ip in &self.allocated_ips {
            content.push_str(&ip.to_string());
            content.push('\n');
        }

        fs::write(&state_file, content)?;
        Ok(())
    }

    /// Load allocation state from disk
    fn load_state(&mut self) -> Result<(), NetworkError> {
        let state_file = Path::new("/var/db/kawakaze").join("ip_allocations.txt");

        if !state_file.exists() {
            return Err(NetworkError::IoError(
                std::io::Error::new(std::io::ErrorKind::NotFound, "State file not found")
            ));
        }

        let content = fs::read_to_string(&state_file)?;
        let mut max_offset = 1;

        for line in content.lines() {
            if let Ok(ip) = line.parse::<std::net::Ipv4Addr>() {
                if self.is_in_network(ip) {
                    let octets = ip.octets();
                    let offset = (octets[2] as u32) * 256 + (octets[3] as u32);
                    if offset > max_offset {
                        max_offset = offset;
                    }
                    self.allocated_ips.insert(ip);
                }
            }
        }

        self.next_ip = max_offset + 1;
        debug!("Loaded {} IP allocations, next is {}", self.allocated_ips.len(), self.next_ip);
        Ok(())
    }

    /// Get the number of allocated IPs
    pub fn allocated_count(&self) -> usize {
        self.allocated_ips.len()
    }
}

impl Default for IpAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Network interface manager
pub struct NetworkManager {
    ip_allocator: IpAllocator,
}

impl NetworkManager {
    /// Create a new network manager
    pub fn new() -> Self {
        Self {
            ip_allocator: IpAllocator::new(),
        }
    }

    /// Initialize the bridge interface and NAT
    pub fn initialize(&self) -> Result<(), NetworkError> {
        info!("Initializing network infrastructure");

        // Check if running as root
        if !is_root() {
            return Err(NetworkError::BridgeCreationFailed(
                "Network initialization requires root privileges".into()
            ));
        }

        #[cfg(target_os = "freebsd")]
        {
            // Create bridge if it doesn't exist
            if !self.bridge_exists()? {
                self.create_bridge()?;
            }

            // Configure NAT with pf
            self.setup_nat()?;

            // Enable IP forwarding
            self.enable_ip_forwarding()?;

            info!("Network infrastructure initialized successfully");
            Ok(())
        }

        #[cfg(not(target_os = "freebsd"))]
        {
            Err(NetworkError::BridgeCreationFailed(
                "Networking is only supported on FreeBSD".into()
            ))
        }
    }

    /// Check if bridge exists
    fn bridge_exists(&self) -> Result<bool, NetworkError> {
        let output = Command::new("ifconfig")
            .arg(BRIDGE_NAME)
            .output()?;

        Ok(output.status.success())
    }

    /// Create the bridge interface
    fn create_bridge(&self) -> Result<(), NetworkError> {
        info!("Creating bridge interface {}", BRIDGE_NAME);

        // Create the bridge
        let output = Command::new("ifconfig")
            .arg(BRIDGE_NAME)
            .arg("create")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::BridgeCreationFailed(format!(
                "Failed to create bridge: {}", stderr
            )));
        }

        // Configure IP address
        let output = Command::new("ifconfig")
            .arg(BRIDGE_NAME)
            .arg("inet")
            .arg(BRIDGE_IP)
            .arg("up")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::BridgeCreationFailed(format!(
                "Failed to configure bridge IP: {}", stderr
            )));
        }

        info!("Bridge {} created with IP {}", BRIDGE_NAME, BRIDGE_IP);
        Ok(())
    }

    /// Enable IP forwarding
    fn enable_ip_forwarding(&self) -> Result<(), NetworkError> {
        debug!("Enabling IP forwarding");

        let output = Command::new("sysctl")
            .arg("net.inet.ip.forwarding=1")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to enable IP forwarding: {}", stderr);
        }

        Ok(())
    }

    /// Set up NAT with pf
    fn setup_nat(&self) -> Result<(), NetworkError> {
        info!("Setting up NAT with pf");

        // Enable pf
        let output = Command::new("pfctl")
            .arg("-e")
            .output();

        // pf might already be enabled, ignore error
        if let Ok(output) = output {
            if !output.status.success() {
                debug!("pf enable warning: {}", String::from_utf8_lossy(&output.stderr));
            }
        }

        // Get the default interface
        let default_iface = self.get_default_interface()?;
        if default_iface.is_none() {
            warn!("Could not determine default interface for NAT");
            return Ok(()); // Continue anyway, user can configure manually
        }

        let default_iface = default_iface.unwrap();

        // Flush existing rules in our anchor
        let _ = Command::new("pfctl")
            .arg("-a")
            .arg(PF_ANCHOR)
            .arg("-F")
            .arg("all")
            .output();

        // Create NAT rules
        // nat on $ext_if from 10.11.0.0/16 to any -> ($ext_if)
        let nat_rules = format!(
            "nat on {} from {} to any -> ({})\n",
            default_iface, NETWORK_CIDR, default_iface
        );

        // Load the rules
        let output = Command::new("pfctl")
            .arg("-a")
            .arg(PF_ANCHOR)
            .arg("-f")
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(nat_rules.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| NetworkError::PfError(format!("Failed to execute pfctl: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::PfError(format!(
                "Failed to load NAT rules: {}", stderr
            )));
        }

        info!("NAT configured on interface {}", default_iface);
        Ok(())
    }

    /// Get the default network interface
    fn get_default_interface(&self) -> Result<Option<String>, NetworkError> {
        let output = Command::new("netstat")
            .arg("-nr")
            .arg("-f")
            .arg("inet")
            .output()?;

        if !output.status.success() {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Find the default route (0.0.0.0 or default)
        for line in stdout.lines() {
            if line.starts_with("default") || line.starts_with("0.0.0.0") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 8 {
                    return Ok(Some(parts[7].to_string())); // Interface is typically the 8th column
                }
            }
        }

        Ok(None)
    }

    /// Allocate network resources for a container
    pub fn allocate_network(&mut self, jail_name: &str) -> Result<ContainerNetwork, NetworkError> {
        // Allocate IP address
        let ip = self.ip_allocator.allocate()?;

        // Create epair interface
        let epair_a = self.create_epair(jail_name)?;

        // Attach epair_a to bridge
        self.attach_to_bridge(&epair_a)?;

        // The epair_b will be moved into the jail
        // epair interfaces are named epair0a/epair0b, so we need to change just the last char
        let epair_b = format!("{}b", &epair_a[..epair_a.len().saturating_sub(1)]);

        debug!("Allocated network for {}: IP={}, epair={}", jail_name, ip, epair_b);

        Ok(ContainerNetwork {
            ip: ip.to_string(),
            bridge: BRIDGE_NAME.to_string(),
            epair_host: epair_a,
            epair_jail: epair_b,
            gateway: BRIDGE_IP.split('/').next().unwrap().to_string(),
        })
    }

    /// Create an epair interface
    fn create_epair(&self, jail_name: &str) -> Result<String, NetworkError> {
        let epair_name = format!("e_{}", &jail_name[jail_name.len().saturating_sub(8)..]);

        let output = Command::new("ifconfig")
            .arg("epair")
            .arg("create")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::EpairCreationFailed(format!(
                "Failed to create epair: {}", stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let epair_a = stdout.trim().to_string();

        debug!("Created epair interface: {}", epair_a);
        Ok(epair_a)
    }

    /// Attach an interface to the bridge
    fn attach_to_bridge(&self, interface: &str) -> Result<(), NetworkError> {
        let output = Command::new("ifconfig")
            .arg(BRIDGE_NAME)
            .arg("addm")
            .arg(interface)
            .arg("up")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::EpairAttachmentFailed(format!(
                "Failed to attach {} to bridge: {}", interface, stderr
            )));
        }

        // Bring up the interface
        let output = Command::new("ifconfig")
            .arg(interface)
            .arg("up")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to bring up {}: {}", interface, stderr);
        }

        debug!("Attached {} to bridge {}", interface, BRIDGE_NAME);
        Ok(())
    }

    /// Move epair interface to VNET jail with retry logic
    ///
    /// This function retries the epair attachment with exponential backoff.
    /// VNET jails may not be fully initialized immediately after creation,
    /// so we wait briefly before the first attempt and retry multiple times.
    /// Note: With ZFS atime=off, VNET initialization is now fast (<1 second).
    fn move_epair_to_vnet_jail(
        &self,
        epair: &str,
        jail_name: &str,
    ) -> Result<(), NetworkError> {
        const MAX_ATTEMPTS: u32 = 10;

        // VNET jails need brief time to initialize after creation
        // With atime=off on ZFS datasets, initialization is now fast
        info!("Waiting briefly for VNET jail {} to initialize...", jail_name);
        thread::sleep(Duration::from_millis(500));

        let mut delay = Duration::from_millis(500); // Start with 500ms delay

        for attempt in 1..=MAX_ATTEMPTS {
            let output = Command::new("ifconfig")
                .arg(epair)
                .arg("-vnet")
                .arg(jail_name)
                .output()?;

            if output.status.success() {
                info!("Successfully moved {} to jail {} on attempt {}", epair, jail_name, attempt);
                return Ok(());
            }

            let stderr = String::from_utf8_lossy(&output.stderr);

            // Check if this is a timing-related error (Device not configured)
            if stderr.contains("Device not configured") && attempt < MAX_ATTEMPTS {
                info!(
                    "Attempt {}/{}: Failed to move {} to jail {} ({}). Retrying after {:?}...",
                    attempt, MAX_ATTEMPTS, epair, jail_name, stderr.trim(), delay
                );
                thread::sleep(delay);
                // Exponential backoff with a cap at 5 seconds
                delay = std::cmp::min(delay * 2, Duration::from_secs(5));
                continue;
            }

            // Either it's not a retryable error or we've exhausted attempts
            return Err(NetworkError::EpairAttachmentFailed(format!(
                "Failed to move {} to jail {} after {} attempt(s): {}",
                epair, jail_name, attempt, stderr
            )));
        }

        unreachable!()
    }

    /// Configure network inside a jail
    pub fn configure_jail_network(
        &self,
        jail_name: &str,
        network: &ContainerNetwork,
    ) -> Result<(), NetworkError> {
        info!("Configuring network for jail {}", jail_name);

        // Note: The epair interface is already moved into the jail via vnet.interface
        // parameter during jail creation. We only need to configure the IP and routing.

        // Configure IP address inside the jail using jexec
        let output = Command::new("jexec")
            .arg(jail_name)
            .arg("ifconfig")
            .arg(&network.epair_jail)
            .arg("inet")
            .arg(&format!("{}/16", network.ip))
            .arg("up")
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::EpairAttachmentFailed(format!(
                "Failed to configure IP in jail {}: {}", jail_name, stderr
            )));
        }

        // Set default route
        let output = Command::new("jexec")
            .arg(jail_name)
            .arg("route")
            .arg("add")
            .arg("default")
            .arg(&network.gateway)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to set default route in jail {}: {}", jail_name, stderr);
        }

        info!("Network configured for jail {}: IP={}, gateway={}",
              jail_name, network.ip, network.gateway);
        Ok(())
    }

    /// Release network resources for a container
    pub fn release_network(&mut self, network: &ContainerNetwork) -> Result<(), NetworkError> {
        // Release IP address
        if let Ok(ip) = network.ip.parse::<std::net::Ipv4Addr>() {
            self.ip_allocator.release(ip)?;
        }

        // Remove epair interfaces
        // Note: epair_b should be automatically removed when the jail stops
        // We just need to remove epair_a from the bridge and destroy it
        let _ = Command::new("ifconfig")
            .arg(BRIDGE_NAME)
            .arg("remm")
            .arg(&network.epair_host)
            .output();

        let _ = Command::new("ifconfig")
            .arg(&network.epair_host)
            .arg("destroy")
            .output();

        debug!("Released network resources: IP={}, epair={}", network.ip, network.epair_host);
        Ok(())
    }

    /// Set up port forwarding for a container
    pub fn setup_port_forwarding(
        &self,
        container_ip: &str,
        host_port: u16,
        container_port: u16,
        protocol: &str,
    ) -> Result<(), NetworkError> {
        info!("Setting up port forwarding: {} -> {}:{} ({})",
              host_port, container_ip, container_port, protocol);

        // rdr pass on $ext_if inet proto tcp from any to any port $host_port -> $container_ip port $container_port
        let rule = format!(
            "rdr pass on {} inet proto {} from any to any port {} -> {} port {}\n",
            BRIDGE_NAME, protocol, host_port, container_ip, container_port
        );

        let output = Command::new("pfctl")
            .arg("-a")
            .arg(format!("{}_forwarding", PF_ANCHOR))
            .arg("-f")
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(rule.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| NetworkError::PfError(format!("Failed to execute pfctl: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(NetworkError::PfError(format!(
                "Failed to load port forwarding rule: {}", stderr
            )));
        }

        info!("Port forwarding configured successfully");
        Ok(())
    }

    /// Remove port forwarding for a container
    pub fn remove_port_forwarding(&self, _container_ip: &str) -> Result<(), NetworkError> {
        // Flush all port forwarding rules
        let _ = Command::new("pfctl")
            .arg("-a")
            .arg(format!("{}_forwarding", PF_ANCHOR))
            .arg("-F")
            .arg("all")
            .output();

        Ok(())
    }
}

impl Default for NetworkManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Network configuration for a container
#[derive(Debug, Clone)]
pub struct ContainerNetwork {
    pub ip: String,
    pub bridge: String,
    pub epair_host: String,
    pub epair_jail: String,
    pub gateway: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_allocator_allocation() {
        let mut allocator = IpAllocator::new();

        let ip1 = allocator.allocate().unwrap();
        let ip2 = allocator.allocate().unwrap();

        assert_ne!(ip1, ip2);
        assert_eq!(allocator.allocated_count(), 2);
    }

    #[test]
    fn test_ip_allocator_specific() {
        let mut allocator = IpAllocator::new();

        let ip = std::net::Ipv4Addr::new(10, 11, 0, 100);
        allocator.allocate_specific(ip).unwrap();

        assert!(allocator.allocated_ips.contains(&ip));
        assert_eq!(allocator.allocated_count(), 1);
    }

    #[test]
    fn test_ip_allocator_specific_out_of_network() {
        let mut allocator = IpAllocator::new();

        let ip = std::net::Ipv4Addr::new(192, 168, 1, 1);
        let result = allocator.allocate_specific(ip);

        assert!(result.is_err());
    }

    #[test]
    fn test_ip_allocator_specific_already_allocated() {
        let mut allocator = IpAllocator::new();

        let ip = std::net::Ipv4Addr::new(10, 11, 0, 50);
        allocator.allocate_specific(ip).unwrap();
        let result = allocator.allocate_specific(ip);

        assert!(result.is_err());
    }

    #[test]
    fn test_ip_allocator_release() {
        let mut allocator = IpAllocator::new();

        let ip = allocator.allocate().unwrap();
        assert_eq!(allocator.allocated_count(), 1);

        allocator.release(ip).unwrap();
        assert_eq!(allocator.allocated_count(), 0);
    }

    #[test]
    fn test_offset_to_ip() {
        let allocator = IpAllocator::new();

        let ip = allocator.offset_to_ip(2).unwrap();
        assert_eq!(ip, std::net::Ipv4Addr::new(10, 11, 0, 2));

        let ip = allocator.offset_to_ip(258).unwrap();
        assert_eq!(ip, std::net::Ipv4Addr::new(10, 11, 1, 2));

        let ip = allocator.offset_to_ip(65534).unwrap();
        assert_eq!(ip, std::net::Ipv4Addr::new(10, 11, 255, 254));
    }

    #[test]
    fn test_offset_to_ip_invalid() {
        let allocator = IpAllocator::new();

        assert!(allocator.offset_to_ip(1).is_err());
        assert!(allocator.offset_to_ip(65535).is_err());
    }

    #[test]
    fn test_is_in_network() {
        let allocator = IpAllocator::new();

        assert!(allocator.is_in_network(std::net::Ipv4Addr::new(10, 11, 0, 1)));
        assert!(allocator.is_in_network(std::net::Ipv4Addr::new(10, 11, 255, 255)));

        assert!(!allocator.is_in_network(std::net::Ipv4Addr::new(10, 12, 0, 1)));
        assert!(!allocator.is_in_network(std::net::Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn test_container_network() {
        let network = ContainerNetwork {
            ip: "10.11.0.2".to_string(),
            bridge: "bridge0".to_string(),
            epair_host: "epair0a".to_string(),
            epair_jail: "epair0b".to_string(),
            gateway: "10.11.0.1".to_string(),
        };

        assert_eq!(network.ip, "10.11.0.2");
        assert_eq!(network.bridge, "bridge0");
        assert_eq!(network.epair_host, "epair0a");
        assert_eq!(network.epair_jail, "epair0b");
        assert_eq!(network.gateway, "10.11.0.1");
    }
}
