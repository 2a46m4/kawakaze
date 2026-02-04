use std::collections::HashMap;
use std::path::PathBuf;
use std::fmt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type ImageId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageState {
    Building,
    Available,
    Deleted,
}

impl ImageState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageState::Building => "building",
            ImageState::Available => "available",
            ImageState::Deleted => "deleted",
        }
    }
}

impl fmt::Display for ImageState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ImageState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "building" => Ok(ImageState::Building),
            "available" => Ok(ImageState::Available),
            "deleted" => Ok(ImageState::Deleted),
            _ => Err(format!("Invalid ImageState: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DockerfileInstruction {
    From(String),
    Run(String),
    Copy { from: Option<String>, src: String, dest: String },
    Add { src: String, dest: String },
    WorkDir(String),
    Env(HashMap<String, String>),
    Expose(Vec<u16>),
    User(String),
    Volume(Vec<String>),
    Cmd(Vec<String>),
    Entrypoint(Vec<String>),
    Label(HashMap<String, String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageConfig {
    pub env: HashMap<String, String>,
    pub workdir: Option<PathBuf>,
    pub user: Option<String>,
    pub exposed_ports: Vec<u16>,
    pub volumes: Vec<String>,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub id: ImageId,
    pub name: String,
    pub parent_id: Option<ImageId>,
    pub snapshot: String,
    pub dockerfile: Vec<DockerfileInstruction>,
    pub config: ImageConfig,
    pub size_bytes: u64,
    pub state: ImageState,
    pub created_at: i64,
}

impl Image {
    pub fn new(name: String, dockerfile: Vec<DockerfileInstruction>) -> Self {
        Self {
            id: Self::generate_id(),
            name,
            parent_id: None,
            snapshot: String::new(),
            dockerfile,
            config: ImageConfig::default(),
            size_bytes: 0,
            state: ImageState::Building,
            created_at: chrono::Utc::now().timestamp(),
        }
    }

    pub fn generate_id() -> ImageId {
        Uuid::new_v4().to_string()
    }

    pub fn with_parent(mut self, parent_id: ImageId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }

    pub fn with_snapshot(mut self, snapshot: String) -> Self {
        self.snapshot = snapshot;
        self
    }

    pub fn with_config(mut self, config: ImageConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_size(mut self, size_bytes: u64) -> Self {
        self.size_bytes = size_bytes;
        self
    }

    pub fn with_state(mut self, state: ImageState) -> Self {
        self.state = state;
        self
    }

    pub fn is_available(&self) -> bool {
        self.state == ImageState::Available
    }

    pub fn is_building(&self) -> bool {
        self.state == ImageState::Building
    }

    pub fn is_deleted(&self) -> bool {
        self.state == ImageState::Deleted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_state_as_str() {
        assert_eq!(ImageState::Building.as_str(), "building");
        assert_eq!(ImageState::Available.as_str(), "available");
        assert_eq!(ImageState::Deleted.as_str(), "deleted");
    }

    #[test]
    fn test_image_state_display() {
        assert_eq!(format!("{}", ImageState::Building), "building");
        assert_eq!(format!("{}", ImageState::Available), "available");
        assert_eq!(format!("{}", ImageState::Deleted), "deleted");
    }

    #[test]
    fn test_image_state_from_str() {
        assert_eq!("building".parse::<ImageState>().unwrap(), ImageState::Building);
        assert_eq!("available".parse::<ImageState>().unwrap(), ImageState::Available);
        assert_eq!("deleted".parse::<ImageState>().unwrap(), ImageState::Deleted);
        assert_eq!("BUILDING".parse::<ImageState>().unwrap(), ImageState::Building);
        assert_eq!("Available".parse::<ImageState>().unwrap(), ImageState::Available);
    }

    #[test]
    fn test_image_state_from_str_invalid() {
        assert!("invalid".parse::<ImageState>().is_err());
        assert!("".parse::<ImageState>().is_err());
    }

    #[test]
    fn test_image_new() {
        let dockerfile = vec![
            DockerfileInstruction::From("freebsd:15.0".to_string()),
            DockerfileInstruction::Run("pkg install -y nginx".to_string()),
        ];

        let image = Image::new("test-image".to_string(), dockerfile.clone());

        assert_eq!(image.name, "test-image");
        assert_eq!(image.dockerfile, dockerfile);
        assert_eq!(image.state, ImageState::Building);
        assert_eq!(image.parent_id, None);
        assert_eq!(image.snapshot, "");
        assert_eq!(image.size_bytes, 0);
        assert!(image.created_at > 0);
        assert!(!image.id.is_empty());
    }

    #[test]
    fn test_image_generate_id() {
        let id1 = Image::generate_id();
        let id2 = Image::generate_id();

        assert_ne!(id1, id2);
        assert!(id1.len() > 0);
        assert!(id2.len() > 0);
    }

    #[test]
    fn test_image_builder_methods() {
        let dockerfile = vec![
            DockerfileInstruction::From("freebsd:15.0".to_string()),
        ];

        let mut config = ImageConfig::default();
        config.env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());

        let image = Image::new("test-image".to_string(), dockerfile)
            .with_parent("parent-123".to_string())
            .with_snapshot("zroot/kawakaze/images/test-image@snap1".to_string())
            .with_config(config.clone())
            .with_size(1024 * 1024 * 500)
            .with_state(ImageState::Available);

        assert_eq!(image.parent_id, Some("parent-123".to_string()));
        assert_eq!(image.snapshot, "zroot/kawakaze/images/test-image@snap1");
        assert_eq!(image.config.env, config.env);
        assert_eq!(image.size_bytes, 1024 * 1024 * 500);
        assert_eq!(image.state, ImageState::Available);
    }

    #[test]
    fn test_image_state_checkers() {
        let mut image = Image::new("test".to_string(), vec![]);

        assert!(image.is_building());
        assert!(!image.is_available());
        assert!(!image.is_deleted());

        image.state = ImageState::Available;
        assert!(!image.is_building());
        assert!(image.is_available());
        assert!(!image.is_deleted());

        image.state = ImageState::Deleted;
        assert!(!image.is_building());
        assert!(!image.is_available());
        assert!(image.is_deleted());
    }

    #[test]
    fn test_dockerfile_instruction_equality() {
        let from1 = DockerfileInstruction::From("freebsd:15.0".to_string());
        let from2 = DockerfileInstruction::From("freebsd:15.0".to_string());
        let from3 = DockerfileInstruction::From("freebsd:14.0".to_string());

        assert_eq!(from1, from2);
        assert_ne!(from1, from3);

        let env1 = DockerfileInstruction::Env({
            let mut map = HashMap::new();
            map.insert("PATH".to_string(), "/usr/bin".to_string());
            map
        });

        let env2 = DockerfileInstruction::Env({
            let mut map = HashMap::new();
            map.insert("PATH".to_string(), "/usr/bin".to_string());
            map
        });

        assert_eq!(env1, env2);
    }

    #[test]
    fn test_image_config_default() {
        let config = ImageConfig::default();

        assert!(config.env.is_empty());
        assert!(config.workdir.is_none());
        assert!(config.user.is_none());
        assert!(config.exposed_ports.is_empty());
        assert!(config.volumes.is_empty());
        assert!(config.entrypoint.is_none());
        assert!(config.cmd.is_none());
        assert!(config.labels.is_empty());
    }

    #[test]
    fn test_image_serialization() {
        let dockerfile = vec![
            DockerfileInstruction::From("freebsd:15.0".to_string()),
            DockerfileInstruction::Run("pkg install -y nginx".to_string()),
            DockerfileInstruction::Expose(vec![80, 443]),
        ];

        let image = Image::new("test-image".to_string(), dockerfile)
            .with_state(ImageState::Available);

        // Test serialization to JSON
        let json = serde_json::to_string(&image).unwrap();
        assert!(json.contains("test-image"));
        assert!(json.contains("available"));

        // Test deserialization from JSON
        let deserialized: Image = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, image.name);
        assert_eq!(deserialized.state, image.state);
        assert_eq!(deserialized.dockerfile, image.dockerfile);
    }

    #[test]
    fn test_dockerfile_instruction_serialization() {
        let instruction = DockerfileInstruction::Copy {
            from: Some("stage1".to_string()),
            src: "/app/src".to_string(),
            dest: "/usr/src".to_string(),
        };

        let json = serde_json::to_string(&instruction).unwrap();
        let deserialized: DockerfileInstruction = serde_json::from_str(&json).unwrap();

        assert_eq!(instruction, deserialized);
    }

    #[test]
    fn test_env_instruction_serialization() {
        let mut env_map = HashMap::new();
        env_map.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        env_map.insert("HOME".to_string(), "/root".to_string());

        let instruction = DockerfileInstruction::Env(env_map.clone());

        let json = serde_json::to_string(&instruction).unwrap();
        let deserialized: DockerfileInstruction = serde_json::from_str(&json).unwrap();

        match deserialized {
            DockerfileInstruction::Env(map) => {
                assert_eq!(map.get("PATH"), Some(&"/usr/bin:/bin".to_string()));
                assert_eq!(map.get("HOME"), Some(&"/root".to_string()));
            }
            _ => panic!("Wrong instruction type"),
        }
    }

    #[test]
    fn test_image_with_complex_config() {
        let mut config = ImageConfig::default();
        config.env.insert("RUST_LOG".to_string(), "info".to_string());
        config.workdir = Some(PathBuf::from("/app"));
        config.user = Some("appuser".to_string());
        config.exposed_ports = vec![8080, 8443];
        config.volumes = vec!["/data".to_string(), "/logs".to_string()];
        config.cmd = Some(vec!["./app".to_string()]);
        config.entrypoint = Some(vec!["/bin/sh".to_string(), "-c".to_string()]);
        config.labels.insert("version".to_string(), "1.0.0".to_string());

        let dockerfile = vec![DockerfileInstruction::From("freebsd:15.0".to_string())];
        let image = Image::new("complex-image".to_string(), dockerfile)
            .with_config(config.clone());

        assert_eq!(image.config.env.get("RUST_LOG"), Some(&"info".to_string()));
        assert_eq!(image.config.workdir, Some(PathBuf::from("/app")));
        assert_eq!(image.config.user, Some("appuser".to_string()));
        assert_eq!(image.config.exposed_ports, vec![8080, 8443]);
        assert_eq!(image.config.volumes, vec!["/data".to_string(), "/logs".to_string()]);
        assert_eq!(image.config.cmd, Some(vec!["./app".to_string()]));
        assert_eq!(
            image.config.entrypoint,
            Some(vec!["/bin/sh".to_string(), "-c".to_string()])
        );
        assert_eq!(image.config.labels.get("version"), Some(&"1.0.0".to_string()));
    }
}
