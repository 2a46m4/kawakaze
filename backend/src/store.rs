//! SQLite persistence layer for jail configurations
//!
//! This module provides database storage for jail configurations,
//! allowing the jail manager to survive restarts and crashes.

use rusqlite::{params, Connection};
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
