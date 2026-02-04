//! SQLite persistence layer for jail configurations
//!
//! This module provides database storage for jail configurations,
//! allowing the jail manager to survive restarts and crashes.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Jail row for database serialization
#[derive(Debug, Clone)]
pub struct JailRow {
    pub name: String,
    pub path: Option<String>,
    pub ip: Option<String>,
    pub state: String,
    pub jid: i32,
}

/// Image state enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageState {
    Building,
    Available,
    Deleted,
}

impl ImageState {
    /// Convert from database string
    pub fn from_str(s: &str) -> Result<Self, StoreError> {
        match s {
            "building" => Ok(ImageState::Building),
            "available" => Ok(ImageState::Available),
            "deleted" => Ok(ImageState::Deleted),
            _ => Err(StoreError::InvalidState(format!("Unknown image state: {}", s))),
        }
    }

    /// Convert to database string
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageState::Building => "building",
            ImageState::Available => "available",
            ImageState::Deleted => "deleted",
        }
    }
}

/// Container state enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    Created,
    Running,
    Stopped,
    Paused,
    Removing,
}

impl ContainerState {
    /// Convert from database string
    pub fn from_str(s: &str) -> Result<Self, StoreError> {
        match s {
            "created" => Ok(ContainerState::Created),
            "running" => Ok(ContainerState::Running),
            "stopped" => Ok(ContainerState::Stopped),
            "paused" => Ok(ContainerState::Paused),
            "removing" => Ok(ContainerState::Removing),
            _ => Err(StoreError::InvalidState(format!("Unknown container state: {}", s))),
        }
    }

    /// Convert to database string
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

/// Image configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageConfig {
    // Placeholder - will be filled in by image.rs
    pub env: Vec<String>,
    pub cmd: Option<String>,
    pub working_dir: Option<String>,
    pub user: Option<String>,
}

/// Image row for database serialization
#[derive(Debug, Clone)]
pub struct Image {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub snapshot: String,
    pub dockerfile: String,  // JSON serialized array of instructions
    pub config: String,       // JSON serialized ImageConfig
    pub size_bytes: i64,
    pub state: ImageState,
    pub created_at: i64,
}

/// Port mapping for containers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,  // "tcp" or "udp"
}

/// Mount configuration for containers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mount {
    pub source: String,
    pub destination: String,
    pub mount_type: String,  // "zfs" or "nullfs"
    pub read_only: bool,
}

/// Container row for database serialization
#[derive(Debug, Clone)]
pub struct Container {
    pub id: String,
    pub name: Option<String>,
    pub image_id: String,
    pub jail_name: String,
    pub dataset: String,
    pub state: ContainerState,
    pub restart_policy: String,
    pub mounts: String,       // JSON serialized array of Mount
    pub port_mappings: String, // JSON serialized array of PortMapping
    pub ip: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
}

/// Store error type
#[derive(Debug)]
pub enum StoreError {
    DatabaseError(rusqlite::Error),
    InvalidState(String),
    SerializationError(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::DatabaseError(e) => write!(f, "Database error: {}", e),
            StoreError::InvalidState(msg) => write!(f, "Invalid state: {}", msg),
            StoreError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::DatabaseError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::DatabaseError(e)
    }
}

// Forward ZFS errors to serialization errors
impl From<crate::zfs::ZfsError> for StoreError {
    fn from(e: crate::zfs::ZfsError) -> Self {
        StoreError::SerializationError(format!("ZFS error: {}", e))
    }
}

/// SQLite persistence store for jails
#[derive(Debug, Clone)]
pub struct JailStore {
    db_path: PathBuf,
}

impl JailStore {
    /// Create a new jail store with the given database path
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        let store = Self { db_path };
        store.init_db()?;
        Ok(store)
    }

    /// Initialize the database schema
    fn init_db(&self) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        // Create jails table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS jails (
                name TEXT PRIMARY KEY,
                path TEXT,
                ip TEXT,
                state TEXT NOT NULL CHECK(state IN ('created', 'running', 'stopped')),
                jid INTEGER,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_jails_state ON jails(state)",
            [],
        )?;

        // Create images table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS images (
                id TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                parent_id TEXT,
                snapshot TEXT NOT NULL,
                dockerfile TEXT NOT NULL,
                config TEXT NOT NULL,
                size_bytes INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL CHECK(state IN ('building', 'available', 'deleted')) DEFAULT 'building',
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (parent_id) REFERENCES images(id) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_images_state ON images(state)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_images_parent ON images(parent_id)",
            [],
        )?;

        // Create containers table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS containers (
                id TEXT PRIMARY KEY,
                name TEXT UNIQUE,
                image_id TEXT NOT NULL,
                jail_name TEXT UNIQUE NOT NULL,
                dataset TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('created', 'running', 'stopped', 'paused', 'removing')) DEFAULT 'created',
                restart_policy TEXT NOT NULL DEFAULT 'no',
                mounts TEXT NOT NULL,
                port_mappings TEXT NOT NULL,
                ip TEXT,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                started_at INTEGER,
                FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE RESTRICT
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_containers_state ON containers(state)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_containers_image ON containers(image_id)",
            [],
        )?;

        debug!("Database initialized at {:?}", self.db_path);
        Ok(())
    }

    /// Insert a new jail into the database
    pub fn insert_jail(&self, jail: &JailRow) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        conn.execute(
            "INSERT INTO jails (name, path, ip, state, jid) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                &jail.name,
                &jail.path,
                &jail.ip,
                &jail.state,
                &jail.jid,
            ],
        )?;

        debug!("Inserted jail '{}' into database", jail.name);
        Ok(())
    }

    /// Update an existing jail in the database
    pub fn update_jail(&self, jail: &JailRow) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        conn.execute(
            "UPDATE jails SET path = ?1, ip = ?2, state = ?3, jid = ?4, updated_at = strftime('%s', 'now') WHERE name = ?5",
            params![
                &jail.path,
                &jail.ip,
                &jail.state,
                &jail.jid,
                &jail.name,
            ],
        )?;

        debug!("Updated jail '{}' in database", jail.name);
        Ok(())
    }

    /// Delete a jail from the database
    pub fn delete_jail(&self, name: &str) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let rows_affected = conn.execute("DELETE FROM jails WHERE name = ?1", params![name])?;

        if rows_affected == 0 {
            warn!("Attempted to delete non-existent jail '{}' from database", name);
        } else {
            debug!("Deleted jail '{}' from database", name);
        }

        Ok(())
    }

    /// Get all jails from the database
    pub fn get_all_jails(&self) -> Result<Vec<JailRow>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT name, path, ip, state, jid FROM jails"
        )?;

        let jail_iter = stmt.query_map([], |row| {
            Ok(JailRow {
                name: row.get(0)?,
                path: row.get(1)?,
                ip: row.get(2)?,
                state: row.get(3)?,
                jid: row.get(4)?,
            })
        })?;

        let mut jails = Vec::new();
        for jail in jail_iter {
            jails.push(jail?);
        }

        debug!("Loaded {} jails from database", jails.len());
        Ok(jails)
    }

    /// Get a single jail by name
    pub fn get_jail(&self, name: &str) -> Result<Option<JailRow>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT name, path, ip, state, jid FROM jails WHERE name = ?1"
        )?;

        let jail_iter = stmt.query_map(params![name], |row| {
            Ok(JailRow {
                name: row.get(0)?,
                path: row.get(1)?,
                ip: row.get(2)?,
                state: row.get(3)?,
                jid: row.get(4)?,
            })
        })?;

        for jail in jail_iter {
            return Ok(Some(jail?));
        }

        Ok(None)
    }

    /// Get the database path
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    // ========== Image Methods ==========

    /// Insert a new image into the database
    pub fn insert_image(&self, image: &Image) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        conn.execute(
            "INSERT INTO images (id, name, parent_id, snapshot, dockerfile, config, size_bytes, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &image.id,
                &image.name,
                &image.parent_id,
                &image.snapshot,
                &image.dockerfile,
                &image.config,
                &image.size_bytes,
                image.state.as_str(),
                &image.created_at,
            ],
        )?;

        debug!("Inserted image '{}' into database", image.name);
        Ok(())
    }

    /// Get an image by ID
    pub fn get_image(&self, id: &str) -> Result<Option<Image>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT id, name, parent_id, snapshot, dockerfile, config, size_bytes, state, created_at
             FROM images WHERE id = ?1"
        )?;

        let image_iter = stmt.query_map(params![id], |row| {
            let state_str: String = row.get(7)?;
            let state = ImageState::from_str(&state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Image {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                snapshot: row.get(3)?,
                dockerfile: row.get(4)?,
                config: row.get(5)?,
                size_bytes: row.get(6)?,
                state,
                created_at: row.get(8)?,
            })
        })?;

        for image in image_iter {
            return Ok(Some(image?));
        }

        Ok(None)
    }

    /// Get an image by name
    pub fn get_image_by_name(&self, name: &str) -> Result<Option<Image>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT id, name, parent_id, snapshot, dockerfile, config, size_bytes, state, created_at
             FROM images WHERE name = ?1"
        )?;

        let image_iter = stmt.query_map(params![name], |row| {
            let state_str: String = row.get(7)?;
            let state = ImageState::from_str(&state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Image {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                snapshot: row.get(3)?,
                dockerfile: row.get(4)?,
                config: row.get(5)?,
                size_bytes: row.get(6)?,
                state,
                created_at: row.get(8)?,
            })
        })?;

        for image in image_iter {
            return Ok(Some(image?));
        }

        Ok(None)
    }

    /// List all images
    pub fn list_images(&self) -> Result<Vec<Image>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT id, name, parent_id, snapshot, dockerfile, config, size_bytes, state, created_at
             FROM images"
        )?;

        let image_iter = stmt.query_map([], |row| {
            let state_str: String = row.get(7)?;
            let state = ImageState::from_str(&state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Image {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                snapshot: row.get(3)?,
                dockerfile: row.get(4)?,
                config: row.get(5)?,
                size_bytes: row.get(6)?,
                state,
                created_at: row.get(8)?,
            })
        })?;

        let mut images = Vec::new();
        for image in image_iter {
            images.push(image?);
        }

        debug!("Loaded {} images from database", images.len());
        Ok(images)
    }

    /// Update an image's state
    pub fn update_image(&self, id: &str, state: ImageState) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let rows_affected = conn.execute(
            "UPDATE images SET state = ?1 WHERE id = ?2",
            params![state.as_str(), id],
        )?;

        if rows_affected == 0 {
            warn!("Attempted to update non-existent image '{}' in database", id);
        } else {
            debug!("Updated image '{}' state to {:?} in database", id, state);
        }

        Ok(())
    }

    /// Delete an image from the database
    pub fn delete_image(&self, id: &str) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let rows_affected = conn.execute("DELETE FROM images WHERE id = ?1", params![id])?;

        if rows_affected == 0 {
            warn!("Attempted to delete non-existent image '{}' from database", id);
        } else {
            debug!("Deleted image '{}' from database", id);
        }

        Ok(())
    }

    // ========== Container Methods ==========

    /// Insert a new container into the database
    pub fn insert_container(&self, container: &Container) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        conn.execute(
            "INSERT INTO containers (id, name, image_id, jail_name, dataset, state, restart_policy, mounts, port_mappings, ip, created_at, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &container.id,
                &container.name,
                &container.image_id,
                &container.jail_name,
                &container.dataset,
                container.state.as_str(),
                &container.restart_policy,
                &container.mounts,
                &container.port_mappings,
                &container.ip,
                &container.created_at,
                &container.started_at,
            ],
        )?;

        debug!("Inserted container '{}' into database", container.id);
        Ok(())
    }

    /// Get a container by ID
    pub fn get_container(&self, id: &str) -> Result<Option<Container>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT id, name, image_id, jail_name, dataset, state, restart_policy, mounts, port_mappings, ip, created_at, started_at
             FROM containers WHERE id = ?1"
        )?;

        let container_iter = stmt.query_map(params![id], |row| {
            let state_str: String = row.get(5)?;
            let state = ContainerState::from_str(&state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Container {
                id: row.get(0)?,
                name: row.get(1)?,
                image_id: row.get(2)?,
                jail_name: row.get(3)?,
                dataset: row.get(4)?,
                state,
                restart_policy: row.get(6)?,
                mounts: row.get(7)?,
                port_mappings: row.get(8)?,
                ip: row.get(9)?,
                created_at: row.get(10)?,
                started_at: row.get(11)?,
            })
        })?;

        for container in container_iter {
            return Ok(Some(container?));
        }

        Ok(None)
    }

    /// Get a container by name
    pub fn get_container_by_name(&self, name: &str) -> Result<Option<Container>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT id, name, image_id, jail_name, dataset, state, restart_policy, mounts, port_mappings, ip, created_at, started_at
             FROM containers WHERE name = ?1"
        )?;

        let container_iter = stmt.query_map(params![name], |row| {
            let state_str: String = row.get(5)?;
            let state = ContainerState::from_str(&state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Container {
                id: row.get(0)?,
                name: row.get(1)?,
                image_id: row.get(2)?,
                jail_name: row.get(3)?,
                dataset: row.get(4)?,
                state,
                restart_policy: row.get(6)?,
                mounts: row.get(7)?,
                port_mappings: row.get(8)?,
                ip: row.get(9)?,
                created_at: row.get(10)?,
                started_at: row.get(11)?,
            })
        })?;

        for container in container_iter {
            return Ok(Some(container?));
        }

        Ok(None)
    }

    /// List all containers
    pub fn list_containers(&self) -> Result<Vec<Container>, StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let mut stmt = conn.prepare(
            "SELECT id, name, image_id, jail_name, dataset, state, restart_policy, mounts, port_mappings, ip, created_at, started_at
             FROM containers"
        )?;

        let container_iter = stmt.query_map([], |row| {
            let state_str: String = row.get(5)?;
            let state = ContainerState::from_str(&state_str)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            Ok(Container {
                id: row.get(0)?,
                name: row.get(1)?,
                image_id: row.get(2)?,
                jail_name: row.get(3)?,
                dataset: row.get(4)?,
                state,
                restart_policy: row.get(6)?,
                mounts: row.get(7)?,
                port_mappings: row.get(8)?,
                ip: row.get(9)?,
                created_at: row.get(10)?,
                started_at: row.get(11)?,
            })
        })?;

        let mut containers = Vec::new();
        for container in container_iter {
            containers.push(container?);
        }

        debug!("Loaded {} containers from database", containers.len());
        Ok(containers)
    }

    /// Update a container's state
    pub fn update_container(&self, id: &str, state: ContainerState) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let rows_affected = conn.execute(
            "UPDATE containers SET state = ?1 WHERE id = ?2",
            params![state.as_str(), id],
        )?;

        if rows_affected == 0 {
            warn!("Attempted to update non-existent container '{}' in database", id);
        } else {
            debug!("Updated container '{}' state to {:?} in database", id, state);
        }

        Ok(())
    }

    /// Delete a container from the database
    pub fn delete_container(&self, id: &str) -> Result<(), StoreError> {
        let conn = Connection::open(&self.db_path)?;

        let rows_affected = conn.execute("DELETE FROM containers WHERE id = ?1", params![id])?;

        if rows_affected == 0 {
            warn!("Attempted to delete non-existent container '{}' from database", id);
        } else {
            debug!("Deleted container '{}' from database", id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_store(test_name: &str) -> JailStore {
        let test_db = format!("/tmp/test_kawakaze_{}.db", test_name);
        let _ = std::fs::remove_file(&test_db); // Clean up any existing test database
        JailStore::new(&test_db).expect("Failed to create test store")
    }

    #[test]
    fn test_store_init() {
        let store = create_test_store("init");
        assert!(store.db_path().exists());
    }

    #[test]
    fn test_insert_and_get_jail() {
        let store = create_test_store("insert_and_get");

        let jail = JailRow {
            name: "test_jail".to_string(),
            path: Some("/tmp/test".to_string()),
            ip: Some("192.168.1.1".to_string()),
            state: "created".to_string(),
            jid: -1,
        };

        store.insert_jail(&jail).unwrap();

        let retrieved = store.get_jail("test_jail").unwrap().unwrap();
        assert_eq!(retrieved.name, "test_jail");
        assert_eq!(retrieved.path, Some("/tmp/test".to_string()));
        assert_eq!(retrieved.ip, Some("192.168.1.1".to_string()));
        assert_eq!(retrieved.state, "created");
        assert_eq!(retrieved.jid, -1);
    }

    #[test]
    fn test_update_jail() {
        let store = create_test_store("update");

        let jail = JailRow {
            name: "test_jail".to_string(),
            path: Some("/tmp/test".to_string()),
            ip: None,
            state: "created".to_string(),
            jid: -1,
        };

        store.insert_jail(&jail).unwrap();

        let mut updated = jail.clone();
        updated.state = "running".to_string();
        updated.jid = 123;

        store.update_jail(&updated).unwrap();

        let retrieved = store.get_jail("test_jail").unwrap().unwrap();
        assert_eq!(retrieved.state, "running");
        assert_eq!(retrieved.jid, 123);
    }

    #[test]
    fn test_delete_jail() {
        let store = create_test_store("delete");

        let jail = JailRow {
            name: "test_jail".to_string(),
            path: None,
            ip: None,
            state: "created".to_string(),
            jid: -1,
        };

        store.insert_jail(&jail).unwrap();
        assert!(store.get_jail("test_jail").unwrap().is_some());

        store.delete_jail("test_jail").unwrap();
        assert!(store.get_jail("test_jail").unwrap().is_none());
    }

    #[test]
    fn test_get_all_jails() {
        let store = create_test_store("get_all");

        let jail1 = JailRow {
            name: "jail1".to_string(),
            path: None,
            ip: None,
            state: "created".to_string(),
            jid: -1,
        };

        let jail2 = JailRow {
            name: "jail2".to_string(),
            path: Some("/tmp/jail2".to_string()),
            ip: Some("10.0.0.1".to_string()),
            state: "running".to_string(),
            jid: 100,
        };

        store.insert_jail(&jail1).unwrap();
        store.insert_jail(&jail2).unwrap();

        let all_jails = store.get_all_jails().unwrap();
        assert_eq!(all_jails.len(), 2);

        let names: Vec<&str> = all_jails.iter().map(|j| j.name.as_str()).collect();
        assert!(names.contains(&"jail1"));
        assert!(names.contains(&"jail2"));
    }

    #[test]
    fn test_duplicate_insert_fails() {
        let store = create_test_store("duplicate");

        let jail = JailRow {
            name: "test_jail".to_string(),
            path: None,
            ip: None,
            state: "created".to_string(),
            jid: -1,
        };

        store.insert_jail(&jail).unwrap();

        let result = store.insert_jail(&jail);
        assert!(result.is_err());
    }
}
