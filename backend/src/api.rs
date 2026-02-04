//! API protocol types for Kawakaze socket communication
//!
//! This module defines the REST-like JSON-over-Unix-socket protocol used for
//! communicating with the Kawakaze jail manager backend.

use crate::jail::{JailError, JailState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// HTTP-like methods for API requests
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    /// Create a resource
    Post,
    /// Retrieve a resource
    Get,
    /// Delete a resource
    Delete,
}

/// API endpoints for jail management
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Endpoint {
    /// List all jails: GET /jails
    Jails,
    /// Get specific jail: GET /jails/{name}
    Jail(String),
    /// Start a jail: POST /jails/{name}/start
    StartJail(String),
    /// Stop a jail: POST /jails/{name}/stop
    StopJail(String),
    /// Bootstrap a jail: POST /jails/{name}/bootstrap
    BootstrapJail(String),
    /// Get bootstrap status: GET /jails/{name}/bootstrap/status
    BootstrapStatus(String),

    // Image endpoints

    /// List all images: GET /images
    Images,
    /// Get specific image: GET /images/{id}
    Image(String),
    /// Build image from Dockerfile: POST /images/build
    ImageBuild,
    /// Delete an image: DELETE /images/{id}
    DeleteImage(String),
    /// Get image history: GET /images/{id}/history
    ImageHistory(String),

    // Container endpoints

    /// List all containers: GET /containers
    Containers,
    /// Get specific container: GET /containers/{id}
    Container(String),
    /// Create container: POST /containers/create
    ContainerCreate,
    /// Start container: POST /containers/{id}/start
    StartContainer(String),
    /// Stop container: POST /containers/{id}/stop
    StopContainer(String),
    /// Remove container: DELETE /containers/{id}
    RemoveContainer(String),
    /// Get container logs: GET /containers/{id}/logs
    ContainerLogs(String),
    /// Execute command in container: POST /containers/{id}/exec
    ContainerExec(String),
}

impl Endpoint {
    /// Get the endpoint path as a string
    pub fn path(&self) -> String {
        match self {
            Endpoint::Jails => "jails".to_string(),
            Endpoint::Jail(name) => format!("jails/{}", name),
            Endpoint::StartJail(name) => format!("jails/{}/start", name),
            Endpoint::StopJail(name) => format!("jails/{}/stop", name),
            Endpoint::BootstrapJail(name) => format!("jails/{}/bootstrap", name),
            Endpoint::BootstrapStatus(name) => format!("jails/{}/bootstrap/status", name),

            Endpoint::Images => "images".to_string(),
            Endpoint::Image(id) => format!("images/{}", id),
            Endpoint::ImageBuild => "images/build".to_string(),
            Endpoint::DeleteImage(id) => format!("images/{}", id),
            Endpoint::ImageHistory(id) => format!("images/{}/history", id),

            Endpoint::Containers => "containers".to_string(),
            Endpoint::Container(id) => format!("containers/{}", id),
            Endpoint::ContainerCreate => "containers/create".to_string(),
            Endpoint::StartContainer(id) => format!("containers/{}/start", id),
            Endpoint::StopContainer(id) => format!("containers/{}/stop", id),
            Endpoint::RemoveContainer(id) => format!("containers/{}", id),
            Endpoint::ContainerLogs(id) => format!("containers/{}/logs", id),
            Endpoint::ContainerExec(id) => format!("containers/{}/exec", id),
        }
    }
}

/// HTTP-like status codes for API responses
pub type StatusCode = u16;

/// Common status codes
pub mod status {
    pub const OK: u16 = 200;
    pub const CREATED: u16 = 201;
    pub const BAD_REQUEST: u16 = 400;
    pub const NOT_FOUND: u16 = 404;
    pub const CONFLICT: u16 = 409;
    pub const INTERNAL_SERVER_ERROR: u16 = 500;
}

/// API request with REST-like method and endpoint
#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
    /// HTTP-like method (GET, POST, DELETE)
    pub method: Method,

    /// API endpoint path
    pub endpoint: String,

    /// Optional request body (as JSON value)
    #[serde(default)]
    pub body: serde_json::Value,
}

impl Request {
    /// Create a new request
    pub fn new(method: Method, endpoint: Endpoint, body: serde_json::Value) -> Self {
        Self {
            method,
            endpoint: endpoint.path(),
            body,
        }
    }

    /// Create a GET request
    pub fn get(endpoint: Endpoint) -> Self {
        Self::new(Method::Get, endpoint, serde_json::Value::Null)
    }

    /// Create a POST request with a body
    pub fn post(endpoint: Endpoint, body: impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            Method::Post,
            endpoint,
            serde_json::to_value(&body)?,
        ))
    }

    /// Create a DELETE request
    pub fn delete(endpoint: Endpoint) -> Self {
        Self::new(Method::Delete, endpoint, serde_json::Value::Null)
    }

    /// Parse the endpoint string into an Endpoint enum
    pub fn parse_endpoint(&self) -> Result<Endpoint, ApiError> {
        // Parse endpoint based on method and path
        let parts: Vec<&str> = self.endpoint.split('/').collect();

        match parts.as_slice() {
            ["jails"] => Ok(Endpoint::Jails),
            ["jails", name] if self.method == Method::Get || self.method == Method::Delete => {
                Ok(Endpoint::Jail(name.to_string()))
            }
            ["jails", name, "start"] => Ok(Endpoint::StartJail(name.to_string())),
            ["jails", name, "stop"] => Ok(Endpoint::StopJail(name.to_string())),
            ["jails", name, "bootstrap"] => Ok(Endpoint::BootstrapJail(name.to_string())),
            ["jails", name, "bootstrap", "status"] => Ok(Endpoint::BootstrapStatus(name.to_string())),

            ["images"] => Ok(Endpoint::Images),
            ["images", "build"] => Ok(Endpoint::ImageBuild),
            ["images", id] if self.method == Method::Get || self.method == Method::Delete => {
                Ok(Endpoint::Image(id.to_string()))
            }
            ["images", id, "history"] => Ok(Endpoint::ImageHistory(id.to_string())),

            ["containers"] => Ok(Endpoint::Containers),
            ["containers", "create"] => Ok(Endpoint::ContainerCreate),
            ["containers", id] if self.method == Method::Get || self.method == Method::Delete => {
                Ok(Endpoint::Container(id.to_string()))
            }
            ["containers", id, "start"] => Ok(Endpoint::StartContainer(id.to_string())),
            ["containers", id, "stop"] => Ok(Endpoint::StopContainer(id.to_string())),
            ["containers", id, "logs"] => Ok(Endpoint::ContainerLogs(id.to_string())),
            ["containers", id, "exec"] => Ok(Endpoint::ContainerExec(id.to_string())),

            _ => Err(ApiError::BadRequest(format!("Unknown endpoint: {}", self.endpoint))),
        }
    }
}

/// API response
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    /// HTTP-like status code
    pub status: StatusCode,

    /// Response data (on success)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,

    /// Error information (on failure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    /// Create a success response with data
    pub fn ok(status: StatusCode, data: impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self {
            status,
            data: Some(serde_json::to_value(data)?),
            error: None,
        })
    }

    /// Create a 200 OK response with data
    pub fn success(data: impl Serialize) -> Result<Self, serde_json::Error> {
        Self::ok(status::OK, data)
    }

    /// Create a 201 Created response with data
    pub fn created(data: impl Serialize) -> Result<Self, serde_json::Error> {
        Self::ok(status::CREATED, data)
    }

    /// Create an error response
    pub fn error(status: StatusCode, error: ApiError) -> Self {
        Self {
            status,
            data: None,
            error: Some(error),
        }
    }

    /// Create a 400 Bad Request error response
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::error(status::BAD_REQUEST, ApiError::BadRequest(message.into()))
    }

    /// Create a 404 Not Found error response
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::error(
            status::NOT_FOUND,
            ApiError::NotFound(resource.into()),
        )
    }

    /// Create a 409 Conflict error response
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::error(status::CONFLICT, ApiError::Conflict(message.into()))
    }

    /// Create a 500 Internal Server Error response
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::error(
            status::INTERNAL_SERVER_ERROR,
            ApiError::Internal(message.into()),
        )
    }

    /// Check if the response indicates success
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }
}

/// API error information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    /// Error code (e.g., "JAIL_NOT_FOUND")
    pub code: String,

    /// Human-readable error message
    pub message: String,
}

impl ApiError {
    /// Create a new API error
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Bad request error (400)
    #[allow(non_snake_case)]
    pub fn BadRequest(message: String) -> Self {
        Self::new("BAD_REQUEST", message)
    }

    /// Not found error (404)
    #[allow(non_snake_case)]
    pub fn NotFound(resource: String) -> Self {
        Self::new("NOT_FOUND", format!("Resource not found: {}", resource))
    }

    /// Conflict error (409)
    #[allow(non_snake_case)]
    pub fn Conflict(message: String) -> Self {
        Self::new("CONFLICT", message)
    }

    /// Internal server error (500)
    #[allow(non_snake_case)]
    pub fn Internal(message: String) -> Self {
        Self::new("INTERNAL_ERROR", message)
    }

    /// Jail already exists error (409)
    #[allow(non_snake_case)]
    pub fn JailAlreadyExists(name: String) -> Self {
        Self::Conflict(format!("Jail '{}' already exists", name))
    }

    /// Jail not found error (404)
    #[allow(non_snake_case)]
    pub fn JailNotFound(name: String) -> Self {
        Self::NotFound(format!("Jail '{}'", name))
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// Convert JailError to ApiError with appropriate status code
impl From<JailError> for ApiError {
    fn from(err: JailError) -> Self {
        match err {
            JailError::CreationFailed(msg) if msg.contains("already exists") => {
                Self::JailAlreadyExists(
                    msg.split('\'')
                        .nth(1)
                        .unwrap_or("unknown")
                        .to_string(),
                )
            }
            JailError::CreationFailed(msg) => Self::BadRequest(msg),
            JailError::StartFailed(msg) if msg.contains("not found") => {
                Self::JailNotFound(
                    msg.split('\'')
                        .nth(1)
                        .unwrap_or("unknown")
                        .to_string(),
                )
            }
            JailError::StartFailed(msg) => Self::new("START_FAILED", msg),
            JailError::StopFailed(msg) if msg.contains("not found") => {
                Self::JailNotFound(
                    msg.split('\'')
                        .nth(1)
                        .unwrap_or("unknown")
                        .to_string(),
                )
            }
            JailError::StopFailed(msg) => Self::new("STOP_FAILED", msg),
            JailError::DestroyFailed(msg) if msg.contains("not found") => {
                Self::JailNotFound(
                    msg.split('\'')
                        .nth(1)
                        .unwrap_or("unknown")
                        .to_string(),
                )
            }
            JailError::DestroyFailed(msg) => Self::new("DESTROY_FAILED", msg),
            JailError::InvalidState(msg) => Self::BadRequest(msg),
            JailError::InvalidPath(msg) => Self::BadRequest(msg),
        }
    }
}

/// Request body for creating a jail
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CreateJailRequest {
    /// Jail name (alphanumeric, underscore, hyphen only)
    pub name: String,

    /// Optional root directory path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Optional IP address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,

    /// Optional bootstrap configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<BootstrapConfig>,
}

impl CreateJailRequest {
    /// Validate the jail creation request
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.name.is_empty() {
            return Err(ApiError::BadRequest("Jail name cannot be empty".into()));
        }

        // Validate jail name (alphanumeric, underscore, and hyphen only)
        if !self.name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(ApiError::BadRequest(
                format!("Invalid jail name '{}': only alphanumeric, underscore, and hyphen characters allowed", self.name)
            ));
        }

        Ok(())
    }
}

/// Jail information in API response format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JailInfo {
    /// Jail name
    pub name: String,

    /// Jail ID (JID), -1 if not running
    pub jid: i32,

    /// Current jail state
    pub state: String,

    /// Root directory path (if set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl From<crate::jail::JailInfo> for JailInfo {
    fn from(info: crate::jail::JailInfo) -> Self {
        Self {
            name: info.name,
            jid: info.jid,
            state: state_to_string(info.state),
            path: info.path,
        }
    }
}

/// Convert JailState to string representation
fn state_to_string(state: JailState) -> String {
    match state {
        JailState::Created => "created".to_string(),
        JailState::Running => "running".to_string(),
        JailState::Stopped => "stopped".to_string(),
    }
}

/// Item in jail list response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JailListItem {
    /// Jail name
    pub name: String,

    /// Current state
    pub state: String,

    /// Whether the jail is running
    pub running: bool,
}

/// Bootstrap configuration (re-exported from bootstrap module)
pub type BootstrapConfig = crate::bootstrap::BootstrapConfig;

/// Bootstrap progress status (re-exported from bootstrap module)
pub type BootstrapProgress = crate::bootstrap::BootstrapProgress;

/// Request body for bootstrapping a jail
pub type BootstrapRequest = BootstrapConfig;

impl From<(String, JailState)> for JailListItem {
    fn from((name, state): (String, JailState)) -> Self {
        Self {
            name,
            state: state_to_string(state),
            running: state == JailState::Running,
        }
    }
}

// ============================================================================
// Image and Container Types
// ============================================================================

/// Placeholder for port mapping (to be implemented in container.rs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String, // "tcp" or "udp"
}

/// Placeholder for volume mount (to be implemented in container.rs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mount {
    pub source: String,
    pub destination: String,
    pub mount_type: String, // "zfs" or "nullfs"
}

// ----------------------------------------------------------------------------
// Image Request Types
// ----------------------------------------------------------------------------

/// Request body for building an image from a Dockerfile
#[derive(Debug, Serialize, Deserialize)]
pub struct BuildImageRequest {
    /// Image name
    pub name: String,
    /// Dockerfile content
    pub dockerfile: String,
    /// Build arguments for Dockerfile ARG instructions
    #[serde(default)]
    pub build_args: HashMap<String, String>,
}

// ----------------------------------------------------------------------------
// Container Request Types
// ----------------------------------------------------------------------------

/// Request body for creating a container
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateContainerRequest {
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
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Restart policy ("on-restart", "on-fail", "noop")
    #[serde(default)]
    pub restart_policy: String,
    /// Optional command to run (overrides image default)
    pub command: Option<Vec<String>>,
}

/// Request body for executing a command in a container
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecRequest {
    /// Command and arguments to execute
    pub command: Vec<String>,
    /// Environment variables for the command
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional working directory
    #[serde(default)]
    pub workdir: Option<String>,
}

// ----------------------------------------------------------------------------
// Image Response Types
// ----------------------------------------------------------------------------

/// Detailed information about an image
#[derive(Debug, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Unique image identifier (UUID)
    pub id: String,
    /// Image name
    pub name: String,
    /// Parent image ID (if built from another image)
    pub parent_id: Option<String>,
    /// Size in bytes
    pub size_bytes: u64,
    /// Image state
    pub state: String,
    /// Unix timestamp of creation
    pub created_at: i64,
}

/// Item in image list response
#[derive(Debug, Serialize, Deserialize)]
pub struct ImageListItem {
    /// Unique image identifier (UUID)
    pub id: String,
    /// Image name
    pub name: String,
    /// Size in bytes
    pub size_bytes: u64,
    /// Unix timestamp of creation
    pub created_at: i64,
}

/// Historical layer information for an image
#[derive(Debug, Serialize, Deserialize)]
pub struct ImageHistoryItem {
    /// Layer ID
    pub id: String,
    /// Created timestamp
    pub created_at: i64,
    /// Size in bytes
    pub size_bytes: u64,
    /// Description/command that created this layer
    pub created_by: String,
}

// ----------------------------------------------------------------------------
// Container Response Types
// ----------------------------------------------------------------------------

/// Detailed information about a container
#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerInfo {
    /// Unique container identifier (UUID)
    pub id: String,
    /// Container name (if set)
    pub name: Option<String>,
    /// Image ID the container is running
    pub image_id: String,
    /// Jail name (internal identifier used by FreeBSD)
    pub jail_name: String,
    /// Container state
    pub state: String,
    /// Container IP address (if running)
    pub ip: Option<String>,
    /// Restart policy
    pub restart_policy: String,
    /// Unix timestamp of creation
    pub created_at: i64,
    /// Unix timestamp when last started
    pub started_at: Option<i64>,
}

/// Item in container list response
#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerListItem {
    /// Unique container identifier (UUID)
    pub id: String,
    /// Container name (if set)
    pub name: Option<String>,
    /// Image ID the container is running
    pub image_id: String,
    /// Container state
    pub state: String,
    /// Container IP address (if running)
    pub ip: Option<String>,
}

/// Container log entry
#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerLogEntry {
    /// Unix timestamp
    pub timestamp: i64,
    /// Log level ("info", "warn", "error", "debug")
    pub level: String,
    /// Log message
    pub message: String,
}

/// Result of executing a command in a container
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecResult {
    /// Exit code
    pub exit_code: i32,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_paths() {
        assert_eq!(Endpoint::Jails.path(), "jails");
        assert_eq!(Endpoint::Jail("test".into()).path(), "jails/test");
        assert_eq!(Endpoint::StartJail("test".into()).path(), "jails/test/start");
        assert_eq!(Endpoint::StopJail("test".into()).path(), "jails/test/stop");

        // Image endpoints
        assert_eq!(Endpoint::Images.path(), "images");
        assert_eq!(Endpoint::Image("abc123".into()).path(), "images/abc123");
        assert_eq!(Endpoint::ImageBuild.path(), "images/build");
        assert_eq!(Endpoint::DeleteImage("abc123".into()).path(), "images/abc123");
        assert_eq!(Endpoint::ImageHistory("abc123".into()).path(), "images/abc123/history");

        // Container endpoints
        assert_eq!(Endpoint::Containers.path(), "containers");
        assert_eq!(Endpoint::Container("def456".into()).path(), "containers/def456");
        assert_eq!(Endpoint::ContainerCreate.path(), "containers/create");
        assert_eq!(Endpoint::StartContainer("def456".into()).path(), "containers/def456/start");
        assert_eq!(Endpoint::StopContainer("def456".into()).path(), "containers/def456/stop");
        assert_eq!(Endpoint::RemoveContainer("def456".into()).path(), "containers/def456");
        assert_eq!(Endpoint::ContainerLogs("def456".into()).path(), "containers/def456/logs");
        assert_eq!(Endpoint::ContainerExec("def456".into()).path(), "containers/def456/exec");
    }

    #[test]
    fn test_request_get() {
        let req = Request::get(Endpoint::Jails);
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.endpoint, "jails");
    }

    #[test]
    fn test_request_post() {
        let body = CreateJailRequest {
            name: "test_jail".into(),
            path: Some("/tmp/test".into()),
            ip: None,
            bootstrap: None,
        };
        let req = Request::post(Endpoint::Jails, body).unwrap();
        assert_eq!(req.method, Method::Post);
        assert_eq!(req.endpoint, "jails");
        assert!(req.body.is_object());
    }

    #[test]
    fn test_request_delete() {
        let req = Request::delete(Endpoint::Jail("test".into()));
        assert_eq!(req.method, Method::Delete);
        assert_eq!(req.endpoint, "jails/test");
    }

    #[test]
    fn test_request_parse_endpoint() {
        let req = Request {
            method: Method::Get,
            endpoint: "jails".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.parse_endpoint().unwrap(), Endpoint::Jails);

        let req = Request {
            method: Method::Get,
            endpoint: "jails/test".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.parse_endpoint().unwrap(), Endpoint::Jail("test".into()));

        let req = Request {
            method: Method::Post,
            endpoint: "jails/test/start".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(
            req.parse_endpoint().unwrap(),
            Endpoint::StartJail("test".into())
        );

        // Image endpoints
        let req = Request {
            method: Method::Get,
            endpoint: "images".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.parse_endpoint().unwrap(), Endpoint::Images);

        let req = Request {
            method: Method::Get,
            endpoint: "images/abc123".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(
            req.parse_endpoint().unwrap(),
            Endpoint::Image("abc123".into())
        );

        let req = Request {
            method: Method::Post,
            endpoint: "images/build".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.parse_endpoint().unwrap(), Endpoint::ImageBuild);

        // Container endpoints
        let req = Request {
            method: Method::Get,
            endpoint: "containers".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.parse_endpoint().unwrap(), Endpoint::Containers);

        let req = Request {
            method: Method::Post,
            endpoint: "containers/create".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(req.parse_endpoint().unwrap(), Endpoint::ContainerCreate);

        let req = Request {
            method: Method::Post,
            endpoint: "containers/def456/start".to_string(),
            body: serde_json::Value::Null,
        };
        assert_eq!(
            req.parse_endpoint().unwrap(),
            Endpoint::StartContainer("def456".into())
        );
    }

    #[test]
    fn test_response_success() {
        let resp = Response::success(serde_json::json!({"test": "data"})).unwrap();
        assert_eq!(resp.status, status::OK);
        assert!(resp.data.is_some());
        assert!(resp.error.is_none());
        assert!(resp.is_success());
    }

    #[test]
    fn test_response_created() {
        let resp = Response::created(serde_json::json!({"name": "test"})).unwrap();
        assert_eq!(resp.status, status::CREATED);
        assert!(resp.is_success());
    }

    #[test]
    fn test_response_error() {
        let resp = Response::not_found("test_jail");
        assert_eq!(resp.status, status::NOT_FOUND);
        assert!(resp.data.is_none());
        assert!(resp.error.is_some());
        assert!(!resp.is_success());
    }

    #[test]
    fn test_create_jail_request_validate() {
        let req = CreateJailRequest {
            name: "valid_name-123".into(),
            path: Some("/tmp/test".into()),
            ip: None,
            bootstrap: None,
        };
        assert!(req.validate().is_ok());

        let req = CreateJailRequest {
            name: "".into(),
            path: None,
            ip: None,
            bootstrap: None,
        };
        assert!(req.validate().is_err());

        let req = CreateJailRequest {
            name: "invalid name!".into(),
            path: None,
            ip: None,
            bootstrap: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_jail_info_conversion() {
        let jail_info = crate::jail::JailInfo {
            name: "test".into(),
            jid: 123,
            state: JailState::Running,
            path: Some("/tmp/test".into()),
        };
        let api_info = JailInfo::from(jail_info);
        assert_eq!(api_info.name, "test");
        assert_eq!(api_info.jid, 123);
        assert_eq!(api_info.state, "running");
        assert_eq!(api_info.path, Some("/tmp/test".into()));
    }

    #[test]
    fn test_api_error_from_jail_error() {
        let err = JailError::CreationFailed("Jail 'test' already exists".into());
        let api_err = ApiError::from(err);
        assert_eq!(api_err.code, "CONFLICT");

        let err = JailError::StartFailed("Jail 'test' not found".into());
        let api_err = ApiError::from(err);
        assert_eq!(api_err.code, "NOT_FOUND");

        let err = JailError::StopFailed("Jail 'missing' not found".into());
        let api_err = ApiError::from(err);
        assert_eq!(api_err.code, "NOT_FOUND");
    }

    #[test]
    fn test_build_image_request() {
        let mut build_args = HashMap::new();
        build_args.insert("VERSION".to_string(), "1.0".to_string());

        let req = BuildImageRequest {
            name: "test-image".to_string(),
            dockerfile: "FROM freebsd:15.0\nRUN pkg install -y nginx".to_string(),
            build_args,
        };

        assert_eq!(req.name, "test-image");
        assert_eq!(req.build_args.len(), 1);
        assert_eq!(req.build_args.get("VERSION"), Some(&"1.0".to_string()));
    }

    #[test]
    fn test_create_container_request() {
        let req = CreateContainerRequest {
            image_id: "abc123".to_string(),
            name: Some("webserver".to_string()),
            ports: vec![PortMapping {
                host_port: 8080,
                container_port: 80,
                protocol: "tcp".to_string(),
            }],
            volumes: vec![Mount {
                source: "/data".to_string(),
                destination: "/mnt/data".to_string(),
                mount_type: "nullfs".to_string(),
            }],
            env: {
                let mut map = HashMap::new();
                map.insert("DEBUG".to_string(), "true".to_string());
                map
            },
            restart_policy: "on-fail".to_string(),
            command: Some(vec!["nginx".to_string(), "-g".to_string(), "daemon off;".to_string()]),
        };

        assert_eq!(req.image_id, "abc123");
        assert_eq!(req.name, Some("webserver".to_string()));
        assert_eq!(req.ports.len(), 1);
        assert_eq!(req.volumes.len(), 1);
        assert_eq!(req.restart_policy, "on-fail");
        assert!(req.command.is_some());
    }

    #[test]
    fn test_exec_request() {
        let req = ExecRequest {
            command: vec!["ls".to_string(), "-la".to_string()],
            env: HashMap::new(),
            workdir: Some("/tmp".to_string()),
        };

        assert_eq!(req.command.len(), 2);
        assert_eq!(req.workdir, Some("/tmp".to_string()));
    }

    #[test]
    fn test_image_info() {
        let info = ImageInfo {
            id: "abc123".to_string(),
            name: "test-image".to_string(),
            parent_id: Some("def456".to_string()),
            size_bytes: 500_000_000,
            state: "ready".to_string(),
            created_at: 1640000000,
        };

        assert_eq!(info.id, "abc123");
        assert_eq!(info.size_bytes, 500_000_000);
        assert!(info.parent_id.is_some());
    }

    #[test]
    fn test_container_info() {
        let info = ContainerInfo {
            id: "container-1".to_string(),
            name: Some("webserver".to_string()),
            image_id: "abc123".to_string(),
            jail_name: "kawakaze-container-1".to_string(),
            state: "running".to_string(),
            ip: Some("10.11.0.2".to_string()),
            restart_policy: "on-restart".to_string(),
            created_at: 1640000000,
            started_at: Some(1640000100),
        };

        assert_eq!(info.id, "container-1");
        assert_eq!(info.state, "running");
        assert_eq!(info.ip, Some("10.11.0.2".to_string()));
        assert!(info.started_at.is_some());
    }
}
