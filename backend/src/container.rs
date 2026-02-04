use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type ContainerId = String;

/// Represents the current state of a container
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    /// Container has been created but not started
    Created,
    /// Container is currently running
    Running,
    /// Container has been stopped
    Stopped,
    /// Container is paused (frozen)
    Paused,
    /// Container is being removed
    Removing,
}

impl ContainerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContainerState::Created => "created",
            ContainerState::Running => "running",
            ContainerState::Stopped => "stopped",
            ContainerState::Paused => "paused",
            ContainerState::Removing => "removing",
        }
    }
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ContainerState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "created" => Ok(ContainerState::Created),
            "running" => Ok(ContainerState::Running),
            "stopped" => Ok(ContainerState::Stopped),
            "paused" => Ok(ContainerState::Paused),
            "removing" => Ok(ContainerState::Removing),
            _ => Err(format!("Invalid container state: {}", s)),
        }
    }
}

/// Defines when a container should be restarted
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartPolicy {
    /// Never restart the container automatically
    No,
    /// Restart only when the system restarts
    OnRestart,
    /// Restart only if the container fails
    OnFailure,
    /// Always restart the container regardless of exit status
    Always,
}

impl RestartPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            RestartPolicy::No => "no",
            RestartPolicy::OnRestart => "on-restart",
            RestartPolicy::OnFailure => "on-failure",
            RestartPolicy::Always => "always",
        }
    }
}

impl std::fmt::Display for RestartPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for RestartPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "no" => Ok(RestartPolicy::No),
            "on-restart" => Ok(RestartPolicy::OnRestart),
            "on-failure" => Ok(RestartPolicy::OnFailure),
            "always" => Ok(RestartPolicy::Always),
            _ => Err(format!("Invalid restart policy: {}", s)),
        }
    }
}

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy::No
    }
}

/// Protocol for port mappings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortProtocol {
    Tcp,
    Udp,
}

impl PortProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            PortProtocol::Tcp => "tcp",
            PortProtocol::Udp => "udp",
        }
    }
}

impl std::fmt::Display for PortProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for PortProtocol {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tcp" => Ok(PortProtocol::Tcp),
            "udp" => Ok(PortProtocol::Udp),
            _ => Err(format!("Invalid port protocol: {}", s)),
        }
    }
}

impl Default for PortProtocol {
    fn default() -> Self {
        PortProtocol::Tcp
    }
}

/// Maps a host port to a container port
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    #[serde(default)]
    pub protocol: PortProtocol,
}

impl PortMapping {
    pub fn new(host_port: u16, container_port: u16, protocol: PortProtocol) -> Self {
        PortMapping {
            host_port,
            container_port,
            protocol,
        }
    }
}

/// Type of mount for a container
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountType {
    /// ZFS dataset mount
    Zfs,
    /// Nullfs (filesystem null mount)
    Nullfs,
}

impl MountType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MountType::Zfs => "zfs",
            MountType::Nullfs => "nullfs",
        }
    }
}

impl std::fmt::Display for MountType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for MountType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "zfs" => Ok(MountType::Zfs),
            "nullfs" => Ok(MountType::Nullfs),
            _ => Err(format!("Invalid mount type: {}", s)),
        }
    }
}

impl Default for MountType {
    fn default() -> Self {
        MountType::Nullfs
    }
}

/// Mount configuration for a container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mount {
    pub source: String,
    pub destination: String,
    #[serde(default)]
    pub mount_type: MountType,
    #[serde(default)]
    pub read_only: bool,
}

impl Mount {
    pub fn new(source: String, destination: String, mount_type: MountType, read_only: bool) -> Self {
        Mount {
            source,
            destination,
            mount_type,
            read_only,
        }
    }
}

/// Configuration for creating a container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    /// Image ID to instantiate
    pub image_id: String,
    /// Optional container name
    pub name: Option<String>,
    /// Port mappings from host to container
    #[serde(default)]
    pub ports: Vec<PortMapping>,
    /// Volume mounts (ZFS datasets or nullfs filepaths)
    #[serde(default)]
    pub volumes: Vec<Mount>,
    /// Restart policy
    #[serde(default)]
    pub restart_policy: RestartPolicy,
}

/// Represents a container (running jail instance)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub id: ContainerId,
    pub name: Option<String>,
    pub image_id: String,
    pub jail_name: String,
    pub dataset: String,
    pub state: ContainerState,
    pub restart_policy: RestartPolicy,
    pub mounts: Vec<Mount>,
    pub port_mappings: Vec<PortMapping>,
    pub ip: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
}

impl Container {
    /// Creates a new container with the given parameters
    pub fn new(image_id: String, jail_name: String, dataset: String) -> Self {
        Container {
            id: Self::generate_id(),
            name: None,
            image_id,
            jail_name,
            dataset,
            state: ContainerState::Created,
            restart_policy: RestartPolicy::default(),
            mounts: Vec::new(),
            port_mappings: Vec::new(),
            ip: None,
            created_at: chrono::Utc::now().timestamp(),
            started_at: None,
        }
    }

    /// Creates a container from existing data (e.g., loaded from database)
    pub fn new_with_existing_data(
        id: ContainerId,
        name: Option<String>,
        image_id: String,
        jail_name: String,
        dataset: String,
        state: ContainerState,
        restart_policy: RestartPolicy,
        mounts: Vec<Mount>,
        port_mappings: Vec<PortMapping>,
        ip: Option<String>,
        created_at: i64,
        started_at: Option<i64>,
    ) -> Self {
        Container {
            id,
            name,
            image_id,
            jail_name,
            dataset,
            state,
            restart_policy,
            mounts,
            port_mappings,
            ip,
            created_at,
            started_at,
        }
    }

    /// Generates a unique container ID using UUID
    pub fn generate_id() -> ContainerId {
        Uuid::new_v4().to_string()
    }

    /// Sets the container name
    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    /// Sets the IP address for the container
    pub fn with_ip(mut self, ip: String) -> Self {
        self.ip = Some(ip);
        self
    }

    /// Sets the restart policy
    pub fn with_restart_policy(mut self, policy: RestartPolicy) -> Self {
        self.restart_policy = policy;
        self
    }

    /// Adds a mount to the container
    pub fn with_mount(mut self, mount: Mount) -> Self {
        self.mounts.push(mount);
        self
    }

    /// Adds a port mapping to the container
    pub fn with_port_mapping(mut self, mapping: PortMapping) -> Self {
        self.port_mappings.push(mapping);
        self
    }

    /// Updates the container state
    pub fn set_state(&mut self, state: ContainerState) {
        self.state = state;
        match state {
            ContainerState::Running => {
                if self.started_at.is_none() {
                    self.started_at = Some(chrono::Utc::now().timestamp());
                }
            }
            ContainerState::Stopped => {
                // Keep started_at for historical info
            }
            _ => {}
        }
    }

    /// Returns whether the container is running
    pub fn is_running(&self) -> bool {
        self.state == ContainerState::Running
    }

    /// Returns whether the container is stopped
    pub fn is_stopped(&self) -> bool {
        self.state == ContainerState::Stopped
    }

    /// Returns a display name for the container (uses name if available, otherwise ID)
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(self.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_state_display() {
        assert_eq!(ContainerState::Created.as_str(), "created");
        assert_eq!(ContainerState::Running.as_str(), "running");
        assert_eq!(ContainerState::Stopped.as_str(), "stopped");
        assert_eq!(ContainerState::Paused.as_str(), "paused");
        assert_eq!(ContainerState::Removing.as_str(), "removing");
    }

    #[test]
    fn test_container_state_from_str() {
        assert_eq!(
            "created".parse::<ContainerState>().unwrap(),
            ContainerState::Created
        );
        assert_eq!(
            "RUNNING".parse::<ContainerState>().unwrap(),
            ContainerState::Running
        );
        assert_eq!(
            "Stopped".parse::<ContainerState>().unwrap(),
            ContainerState::Stopped
        );
    }

    #[test]
    fn test_container_state_invalid() {
        assert!("invalid".parse::<ContainerState>().is_err());
    }

    #[test]
    fn test_restart_policy_display() {
        assert_eq!(RestartPolicy::No.as_str(), "no");
        assert_eq!(RestartPolicy::OnRestart.as_str(), "on-restart");
        assert_eq!(RestartPolicy::OnFailure.as_str(), "on-failure");
        assert_eq!(RestartPolicy::Always.as_str(), "always");
    }

    #[test]
    fn test_restart_policy_from_str() {
        assert_eq!("no".parse::<RestartPolicy>().unwrap(), RestartPolicy::No);
        assert_eq!(
            "ON-RESTART".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnRestart
        );
        assert_eq!(
            "On-Failure".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnFailure
        );
        assert_eq!(
            "always".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::Always
        );
    }

    #[test]
    fn test_restart_policy_default() {
        assert_eq!(RestartPolicy::default(), RestartPolicy::No);
    }

    #[test]
    fn test_restart_policy_invalid() {
        assert!("invalid".parse::<RestartPolicy>().is_err());
    }

    #[test]
    fn test_port_protocol_display() {
        assert_eq!(PortProtocol::Tcp.as_str(), "tcp");
        assert_eq!(PortProtocol::Udp.as_str(), "udp");
    }

    #[test]
    fn test_port_protocol_from_str() {
        assert_eq!("tcp".parse::<PortProtocol>().unwrap(), PortProtocol::Tcp);
        assert_eq!("UDP".parse::<PortProtocol>().unwrap(), PortProtocol::Udp);
    }

    #[test]
    fn test_port_protocol_default() {
        assert_eq!(PortProtocol::default(), PortProtocol::Tcp);
    }

    #[test]
    fn test_port_protocol_invalid() {
        assert!("invalid".parse::<PortProtocol>().is_err());
    }

    #[test]
    fn test_mount_type_display() {
        assert_eq!(MountType::Zfs.as_str(), "zfs");
        assert_eq!(MountType::Nullfs.as_str(), "nullfs");
    }

    #[test]
    fn test_mount_type_from_str() {
        assert_eq!("zfs".parse::<MountType>().unwrap(), MountType::Zfs);
        assert_eq!("NULLFS".parse::<MountType>().unwrap(), MountType::Nullfs);
    }

    #[test]
    fn test_mount_type_default() {
        assert_eq!(MountType::default(), MountType::Nullfs);
    }

    #[test]
    fn test_mount_type_invalid() {
        assert!("invalid".parse::<MountType>().is_err());
    }

    #[test]
    fn test_container_creation() {
        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        );

        assert_eq!(container.image_id, "image-123");
        assert_eq!(container.jail_name, "jail-test");
        assert_eq!(container.dataset, "zroot/jails/test");
        assert_eq!(container.state, ContainerState::Created);
        assert_eq!(container.restart_policy, RestartPolicy::No);
        assert!(container.mounts.is_empty());
        assert!(container.port_mappings.is_empty());
        assert!(container.ip.is_none());
        assert!(container.started_at.is_none());
        assert!(!container.id.is_empty());
    }

    #[test]
    fn test_container_with_name() {
        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        )
        .with_name("my-container".to_string());

        assert_eq!(container.name, Some("my-container".to_string()));
    }

    #[test]
    fn test_container_with_ip() {
        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        )
        .with_ip("10.11.0.2".to_string());

        assert_eq!(container.ip, Some("10.11.0.2".to_string()));
    }

    #[test]
    fn test_container_with_restart_policy() {
        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        )
        .with_restart_policy(RestartPolicy::Always);

        assert_eq!(container.restart_policy, RestartPolicy::Always);
    }

    #[test]
    fn test_container_with_mount() {
        let mount = Mount::new(
            "/host/path".to_string(),
            "/container/path".to_string(),
            MountType::Nullfs,
            false,
        );

        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        )
        .with_mount(mount);

        assert_eq!(container.mounts.len(), 1);
        assert_eq!(container.mounts[0].source, "/host/path");
        assert_eq!(container.mounts[0].destination, "/container/path");
    }

    #[test]
    fn test_container_with_port_mapping() {
        let port_mapping = PortMapping::new(8080, 80, PortProtocol::Tcp);

        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        )
        .with_port_mapping(port_mapping);

        assert_eq!(container.port_mappings.len(), 1);
        assert_eq!(container.port_mappings[0].host_port, 8080);
        assert_eq!(container.port_mappings[0].container_port, 80);
    }

    #[test]
    fn test_container_set_state() {
        let mut container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        );

        assert_eq!(container.state, ContainerState::Created);
        assert!(!container.is_running());

        container.set_state(ContainerState::Running);
        assert_eq!(container.state, ContainerState::Running);
        assert!(container.is_running());
        assert!(container.started_at.is_some());

        container.set_state(ContainerState::Stopped);
        assert_eq!(container.state, ContainerState::Stopped);
        assert!(container.is_stopped());
    }

    #[test]
    fn test_container_display_name() {
        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        );

        // Without name, should display ID
        assert_eq!(container.display_name(), container.id);

        let container = container.with_name("my-container".to_string());

        // With name, should display name
        assert_eq!(container.display_name(), "my-container");
    }

    #[test]
    fn test_container_id_unique() {
        let id1 = Container::generate_id();
        let id2 = Container::generate_id();

        assert_ne!(id1, id2);
    }

    #[test]
    fn test_port_mapping_new() {
        let mapping = PortMapping::new(8080, 80, PortProtocol::Tcp);

        assert_eq!(mapping.host_port, 8080);
        assert_eq!(mapping.container_port, 80);
        assert_eq!(mapping.protocol, PortProtocol::Tcp);
    }

    #[test]
    fn test_mount_new() {
        let mount = Mount::new(
            "/host/path".to_string(),
            "/container/path".to_string(),
            MountType::Zfs,
            true,
        );

        assert_eq!(mount.source, "/host/path");
        assert_eq!(mount.destination, "/container/path");
        assert_eq!(mount.mount_type, MountType::Zfs);
        assert!(mount.read_only);
    }

    #[test]
    fn test_container_serialization() {
        let container = Container::new(
            "image-123".to_string(),
            "jail-test".to_string(),
            "zroot/jails/test".to_string(),
        )
        .with_name("test-container".to_string())
        .with_ip("10.11.0.2".to_string())
        .with_restart_policy(RestartPolicy::Always);

        let json = serde_json::to_string(&container).unwrap();
        let deserialized: Container = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, container.id);
        assert_eq!(deserialized.name, container.name);
        assert_eq!(deserialized.image_id, container.image_id);
        assert_eq!(deserialized.jail_name, container.jail_name);
        assert_eq!(deserialized.dataset, container.dataset);
        assert_eq!(deserialized.state, container.state);
        assert_eq!(deserialized.restart_policy, container.restart_policy);
        assert_eq!(deserialized.ip, container.ip);
    }
}
