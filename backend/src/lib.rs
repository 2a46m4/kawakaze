//! Kawakaze backend - Jail management service
//!
//! This module handles the actual management of FreeBSD jails,
//! communicating with clients through a unix socket.

pub mod jail;
pub mod api;
pub mod handler;
pub mod server;
pub mod store;
pub mod bootstrap;
pub mod config;
pub mod zfs;
pub mod image;
pub mod container;
pub mod image_builder;

use crate::jail::{Jail, JailError, JailState};
use crate::store::{JailStore, StoreError};
use crate::bootstrap::{BootstrapProgress, BootstrapStatus};
use crate::image::{Image, ImageId};
use crate::container::{Container, ContainerId};
use crate::zfs::Zfs;
use crate::config::KawakazeConfig;
use crate::image_builder::ImageBuildProgress;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Type for bootstrap progress sender
pub type BootstrapProgressSender = mpsc::Sender<BootstrapProgress>;

/// Jail manager - handles jail lifecycle
pub struct JailManager {
    pub(crate) socket_path: PathBuf,
    pub(crate) jails: HashMap<String, Jail>,
    running: bool,
    store: Option<JailStore>,
    /// Bootstrap progress trackers (jail name -> progress sender)
    pub bootstrap_tracker: HashMap<String, BootstrapProgressSender>,
    /// Bootstrap progress state (jail name -> latest progress)
    pub bootstrap_progress: HashMap<String, BootstrapProgress>,
    /// Image storage (image ID -> Image)
    pub(crate) images: HashMap<ImageId, Image>,
    /// Container storage (container ID -> Container)
    pub(crate) containers: HashMap<ContainerId, Container>,
    /// ZFS wrapper for dataset management
    pub(crate) zfs: Option<Zfs>,
    /// Configuration
    pub(crate) config: KawakazeConfig,
    /// Image build progress trackers (image ID -> progress sender)
    pub image_build_tracker: HashMap<ImageId, mpsc::Sender<ImageBuildProgress>>,
    /// Image build progress state (image ID -> latest progress)
    pub image_build_progress: HashMap<ImageId, ImageBuildProgress>,
}

impl JailManager {
    /// Create a new jail manager
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            jails: HashMap::new(),
            running: false,
            store: None,
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
            images: HashMap::new(),
            containers: HashMap::new(),
            zfs: None,
            config: KawakazeConfig::default(),
            image_build_tracker: HashMap::new(),
            image_build_progress: HashMap::new(),
        }
    }

    /// Create a jail manager with default socket path
    pub fn with_default_socket() -> Self {
        Self::new("/var/run/kawakaze.sock")
    }

    /// Create a jail manager with database persistence
    pub fn with_database(db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let store = JailStore::new(db_path)?;

        Ok(Self {
            socket_path: PathBuf::from("/var/run/kawakaze.sock"),
            jails: HashMap::new(),
            running: false,
            store: Some(store),
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
            images: HashMap::new(),
            containers: HashMap::new(),
            zfs: None,
            config: KawakazeConfig::default(),
            image_build_tracker: HashMap::new(),
            image_build_progress: HashMap::new(),
        })
    }

    /// Create a jail manager with database persistence at default location
    pub fn with_default_database() -> Result<Self, StoreError> {
        Self::with_database("/var/db/kawakaze.db")
    }

    /// Create a jail manager with custom socket and database paths
    pub fn with_paths(socket_path: impl Into<PathBuf>, db_path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let store = JailStore::new(db_path)?;

        Ok(Self {
            socket_path: socket_path.into(),
            jails: HashMap::new(),
            running: false,
            store: Some(store),
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
            images: HashMap::new(),
            containers: HashMap::new(),
            zfs: None,
            config: KawakazeConfig::default(),
            image_build_tracker: HashMap::new(),
            image_build_progress: HashMap::new(),
        })
    }

    /// Create a jail manager with configuration
    pub fn with_config(config: KawakazeConfig) -> Result<Self, StoreError> {
        let zfs = Zfs::new(&config.zfs_pool).ok();

        // Initialize database with new tables
        let store = JailStore::new(&config.storage.database_path)?;

        Ok(Self {
            socket_path: PathBuf::from(&config.storage.socket_path),
            jails: HashMap::new(),
            running: false,
            store: Some(store),
            bootstrap_tracker: HashMap::new(),
            bootstrap_progress: HashMap::new(),
            images: HashMap::new(),
            containers: HashMap::new(),
            zfs,
            config,
            image_build_tracker: HashMap::new(),
            image_build_progress: HashMap::new(),
        })
    }

    /// Start the jail manager service
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.running {
            return Err("JailManager is already running".into());
        }

        // Load jails from database if configured
        if let Some(ref store) = self.store.clone() {
            self.load_jails_from_db(store)?;
            self.load_images_from_db(store)?;
            self.load_containers_from_db(store)?;
        }

        // Mark as running
        self.running = true;
        Ok(())
    }

    /// Load jails from database and sync JIDs with FreeBSD kernel
    fn load_jails_from_db(&mut self, store: &JailStore) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading jails from database: {:?}", store.db_path());

        let jail_rows = store.get_all_jails()?;
        let mut loaded_count = 0;

        for row in jail_rows {
            let name = row.name.clone();
            match Jail::from_db_row(row) {
                Ok(mut jail) => {
                    // Reset JID to -1 before syncing with kernel
                    jail.set_jid(-1);

                    // Sync with FreeBSD kernel - check if jail is actually running
                    #[cfg(target_os = "freebsd")]
                    {
                        let actual_jid = self.get_jid_from_kernel(jail.name());
                        if let Some(jid) = actual_jid {
                            jail.set_jid(jid);
                            jail.set_state(JailState::Running);
                            debug!("Jail '{}' is running with JID {}", jail.name(), jid);
                        } else {
                            // Jail is not running, ensure state is Stopped or Created
                            if jail.state() == JailState::Running {
                                jail.set_state(JailState::Stopped);
                            }
                            debug!("Jail '{}' is not running", jail.name());
                        }
                    }

                    self.jails.insert(name.clone(), jail);
                    loaded_count += 1;
                }
                Err(e) => {
                    warn!("Failed to load jail '{}' from database: {}", name, e);
                }
            }
        }

        info!("Loaded {} jails from database", loaded_count);
        Ok(())
    }

    /// Load images from database
    fn load_images_from_db(&mut self, store: &JailStore) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading images from database: {:?}", store.db_path());

        let image_rows = store.list_images()?;
        let mut loaded_count = 0;

        for store_image in image_rows {
            let id = store_image.id.clone();
            match self.load_image_from_store_row(store_image) {
                Ok(image) => {
                    self.images.insert(id.clone(), image);
                    loaded_count += 1;
                }
                Err(e) => {
                    warn!("Failed to load image '{}' from database: {}", id, e);
                }
            }
        }

        info!("Loaded {} images from database", loaded_count);
        Ok(())
    }

    /// Load containers from database
    fn load_containers_from_db(&mut self, store: &JailStore) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading containers from database: {:?}", store.db_path());

        let container_rows = store.list_containers()?;
        let mut loaded_count = 0;

        for store_container in container_rows {
            let id = store_container.id.clone();
            match self.load_container_from_store_row(store_container) {
                Ok(container) => {
                    self.containers.insert(id.clone(), container);
                    loaded_count += 1;
                }
                Err(e) => {
                    warn!("Failed to load container '{}' from database: {}", id, e);
                }
            }
        }

        info!("Loaded {} containers from database", loaded_count);
        Ok(())
    }

    /// Convert a store::Image to an image::Image
    fn load_image_from_store_row(&self, store_image: crate::store::Image) -> Result<crate::image::Image, Box<dyn std::error::Error>> {
        use crate::image::{Image, ImageConfig};

        // Parse dockerfile from JSON
        let dockerfile: Vec<crate::image::DockerfileInstruction> = serde_json::from_str(&store_image.dockerfile)
            .map_err(|e| format!("Failed to parse dockerfile: {}", e))?;

        // Parse config from JSON
        let config: ImageConfig = serde_json::from_str(&store_image.config)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        Ok(Image {
            id: store_image.id,
            name: store_image.name,
            parent_id: store_image.parent_id,
            snapshot: store_image.snapshot,
            dockerfile,
            config,
            size_bytes: store_image.size_bytes as u64,
            state: match store_image.state {
                crate::store::ImageState::Building => crate::image::ImageState::Building,
                crate::store::ImageState::Available => crate::image::ImageState::Available,
                crate::store::ImageState::Deleted => crate::image::ImageState::Deleted,
            },
            created_at: store_image.created_at,
        })
    }

    /// Convert a store::Container to a container::Container
    fn load_container_from_store_row(&self, store_container: crate::store::Container) -> Result<crate::container::Container, Box<dyn std::error::Error>> {
        use crate::container::{Container, ContainerState, RestartPolicy};

        // Parse mounts from JSON
        let mounts: Vec<crate::container::Mount> = serde_json::from_str(&store_container.mounts)
            .map_err(|e| format!("Failed to parse mounts: {}", e))?;

        // Parse port mappings from JSON
        let port_mappings: Vec<crate::container::PortMapping> = serde_json::from_str(&store_container.port_mappings)
            .map_err(|e| format!("Failed to parse port_mappings: {}", e))?;

        let restart_policy = store_container.restart_policy.parse::<RestartPolicy>()
            .map_err(|e| format!("Failed to parse restart_policy: {}", e))?;

        let state = match store_container.state {
            crate::store::ContainerState::Created => ContainerState::Created,
            crate::store::ContainerState::Running => ContainerState::Running,
            crate::store::ContainerState::Stopped => ContainerState::Stopped,
            crate::store::ContainerState::Paused => ContainerState::Paused,
            crate::store::ContainerState::Removing => ContainerState::Removing,
        };

        // Note: We can't reconstruct the full Container with ContainerConfig, so we create a minimal one
        // The actual Container type doesn't have a simple from_db_row method like Jail
        // For now, we'll store minimal info and reconstruct as needed

        // Actually, looking at the Container structure, it doesn't have a simple constructor
        // Let me check how containers are created...

        // For now, let's just store the essential info - the full container loading will need
        // the Container::new constructor which we can't easily call here without refactoring

        // As a workaround, let's create a container using the available info
        Ok(Container::new_with_existing_data(
            store_container.id,
            store_container.name,
            store_container.image_id,
            store_container.jail_name,
            store_container.dataset,
            state,
            restart_policy,
            mounts,
            port_mappings,
            store_container.ip,
            store_container.created_at,
            store_container.started_at,
        ))
    }

    /// Query FreeBSD kernel for JID by jail name
    #[cfg(target_os = "freebsd")]
    fn get_jid_from_kernel(&self, name: &str) -> Option<i32> {
        use std::ffi::CString;
        use std::mem;

        let name_param = CString::new("name").unwrap();
        let name_value = CString::new(name).ok()?;
        let jid_param = CString::new("jid").unwrap();

        let mut jid_out: libc::c_int = 0;

        let iovs = [
            libc::iovec {
                iov_base: name_param.as_ptr() as *mut libc::c_void,
                iov_len: name_param.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: name_value.as_ptr() as *mut libc::c_void,
                iov_len: name_value.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: jid_param.as_ptr() as *mut libc::c_void,
                iov_len: jid_param.as_bytes().len() + 1,
            },
            libc::iovec {
                iov_base: &mut jid_out as *mut libc::c_int as *mut libc::c_void,
                iov_len: mem::size_of::<libc::c_int>(),
            },
        ];

        let result = unsafe {
            libc::jail_get(iovs.as_ptr() as *mut libc::iovec, iovs.len() as libc::c_uint, 0)
        };

        if result > 0 {
            Some(result)
        } else {
            None
        }
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

        // Persist to database if configured
        if let Some(ref store) = self.store {
            let row = jail.to_db_row();
            if let Err(e) = store.insert_jail(&row) {
                error!("Failed to persist jail '{}' to database: {}", name, e);
                // Continue anyway - persistence failures shouldn't block jail operations
            }
        }

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

        jail.start()?;

        // Persist state change to database if configured
        if let Some(ref store) = self.store {
            let row = jail.to_db_row();
            if let Err(e) = store.update_jail(&row) {
                error!("Failed to persist jail '{}' state to database: {}", name, e);
            }
        }

        Ok(())
    }

    /// Stop a jail by name
    pub fn stop_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .get_mut(name)
            .ok_or_else(|| JailError::StopFailed(format!("Jail '{}' not found", name)))?;

        jail.stop()?;

        // Persist state change to database if configured
        if let Some(ref store) = self.store {
            let row = jail.to_db_row();
            if let Err(e) = store.update_jail(&row) {
                error!("Failed to persist jail '{}' state to database: {}", name, e);
            }
        }

        Ok(())
    }

    /// Remove a jail by name
    pub fn remove_jail(&mut self, name: &str) -> Result<(), JailError> {
        let jail = self
            .jails
            .remove(name)
            .ok_or_else(|| JailError::DestroyFailed(format!("Jail '{}' not found", name)))?;

        jail.destroy()?;

        // Remove from database if configured
        if let Some(ref store) = self.store {
            if let Err(e) = store.delete_jail(name) {
                error!("Failed to delete jail '{}' from database: {}", name, e);
            }
        }

        Ok(())
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

    /// Register a bootstrap progress tracker for a jail
    pub async fn register_bootstrap_tracker(&mut self, name: String, sender: BootstrapProgressSender) {
        // Store the sender for later use
        self.bootstrap_tracker.insert(name.clone(), sender);

        // Initialize progress with defaults
        self.bootstrap_progress.insert(name, BootstrapProgress {
            status: BootstrapStatus::Initializing,
            progress: 0,
            current_step: "Bootstrap starting...".to_string(),
            version: "unknown".to_string(),
            architecture: "unknown".to_string(),
        });
    }

    /// Get the current bootstrap progress for a jail
    pub async fn get_bootstrap_progress(&self, name: &str) -> Option<BootstrapProgress> {
        self.bootstrap_progress.get(name).cloned()
    }

    /// Send a bootstrap progress update
    pub async fn send_bootstrap_progress(&mut self, name: &str, status: BootstrapStatus) -> Result<(), mpsc::error::SendError<BootstrapProgress>> {
        if let Some(progress) = self.bootstrap_progress.get_mut(name) {
            progress.status = status.clone();
            match &status {
                BootstrapStatus::Complete => progress.progress = 100,
                BootstrapStatus::Failed(_) => progress.progress = 0,
                _ => {}
            }
            progress.current_step = match &status {
                BootstrapStatus::Complete => "Bootstrap completed".to_string(),
                BootstrapStatus::Failed(msg) => format!("Bootstrap failed: {}", msg),
                _ => "In progress".to_string(),
            };

            // Send update through the channel
            if let Some(sender) = self.bootstrap_tracker.get(name) {
                let _ = sender.try_send(progress.clone());
            }

            Ok(())
        } else {
            Err(mpsc::error::SendError(BootstrapProgress {
                status,
                progress: 0,
                current_step: "".to_string(),
                version: "".to_string(),
                architecture: "".to_string(),
            }))
        }
    }

    /// Remove bootstrap tracker for a jail
    pub async fn remove_bootstrap_tracker(&mut self, name: &str) {
        self.bootstrap_tracker.remove(name);
        self.bootstrap_progress.remove(name);
    }

    // Image management methods

    /// Add an image to the manager
    pub fn add_image(&mut self, image: Image) -> Result<(), StoreError> {
        if let Some(ref store) = self.store {
            // Convert to store Image format
            let store_image = crate::store::Image {
                id: image.id.clone(),
                name: image.name.clone(),
                parent_id: image.parent_id.clone(),
                snapshot: image.snapshot.clone(),
                dockerfile: serde_json::to_string(&image.dockerfile)
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?,
                config: serde_json::to_string(&image.config)
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?,
                size_bytes: image.size_bytes as i64,
                state: crate::store::ImageState::Available, // Since it's being added
                created_at: image.created_at,
            };
            store.insert_image(&store_image)?;
        }

        self.images.insert(image.id.clone(), image);
        Ok(())
    }

    /// Get an image by ID
    pub fn get_image(&self, id: &ImageId) -> Option<&Image> {
        self.images.get(id).or_else(|| {
            // Try to load from database if not in memory
            if let Some(ref store) = self.store {
                if let Ok(Some(store_image)) = store.get_image(id) {
                    // Note: In a real implementation, you'd want to cache this
                    // For now, we'll just return None since we can't convert back easily here
                    return None;
                }
            }
            None
        })
    }

    /// Get an image by name
    pub fn get_image_by_name(&self, name: &str) -> Option<&Image> {
        self.images.values().find(|i| i.name == name).or_else(|| {
            // Try to load from database if not in memory
            if let Some(ref store) = self.store {
                if let Ok(Some(store_image)) = store.get_image_by_name(name) {
                    // Note: Same limitation as above
                    return None;
                }
            }
            None
        })
    }

    /// List all images
    pub fn list_images(&self) -> Vec<&Image> {
        let mut images: Vec<&Image> = self.images.values().collect();

        // Also include images from database
        if let Some(ref store) = self.store {
            if let Ok(db_images) = store.list_images() {
                // Note: In a real implementation, you'd want to convert and deduplicate
                // For now, we'll just return the in-memory images
                debug!("Found {} images in database (not yet loaded)", db_images.len());
            }
        }

        images
    }

    /// Remove an image
    pub fn remove_image(&mut self, id: &ImageId) -> Result<(), StoreError> {
        if let Some(image) = self.get_image(id) {
            // Clean up ZFS snapshot
            if let Some(ref zfs) = self.zfs {
                let _ = zfs.destroy(&image.snapshot);
            }
        }

        if let Some(ref store) = self.store {
            store.delete_image(id)?;
        }

        self.images.remove(id);
        Ok(())
    }

    // Container management methods

    /// Create a container from an image
    pub fn create_container(&mut self, config: crate::container::ContainerConfig) -> Result<Container, StoreError> {
        // Validate image exists
        let image = self.get_image(&config.image_id)
            .ok_or_else(|| StoreError::SerializationError(format!("Image {} not found", config.image_id)))?;

        // Generate container ID
        let container_id = Container::generate_id();
        let jail_name = format!("kawakaze-{}", &container_id[..8]);
        let dataset = format!("{}/containers/{}", self.config.zfs_pool, &container_id[..8]);

        // Create ZFS clone from image snapshot
        if let Some(ref zfs) = self.zfs {
            zfs.clone_snapshot(&image.snapshot, &dataset)?;
        }

        // Create container
        let container = Container::new(config.image_id.clone(), jail_name, dataset)
            .with_name(config.name.unwrap_or_else(|| container_id.clone()))
            .with_restart_policy(config.restart_policy);

        // Store in database
        if let Some(ref store) = self.store {
            let store_container = crate::store::Container {
                id: container.id.clone(),
                name: container.name.clone(),
                image_id: container.image_id.clone(),
                jail_name: container.jail_name.clone(),
                dataset: container.dataset.clone(),
                state: crate::store::ContainerState::Created,
                restart_policy: container.restart_policy.as_str().to_string(),
                mounts: serde_json::to_string(&container.mounts)
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?,
                port_mappings: serde_json::to_string(&container.port_mappings)
                    .map_err(|e| StoreError::SerializationError(e.to_string()))?,
                ip: container.ip.clone(),
                created_at: container.created_at,
                started_at: container.started_at,
            };
            store.insert_container(&store_container)?;
        }

        self.containers.insert(container_id.clone(), container.clone());
        Ok(container)
    }

    /// Start a container
    pub fn start_container(&mut self, id: &ContainerId) -> Result<(), StoreError> {
        // Get the jail name first
        let jail_name = self.containers.get(id)
            .ok_or_else(|| StoreError::SerializationError(format!("Container {} not found", id)))?
            .jail_name.clone();

        // Start the jail
        self.start_jail(&jail_name)
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        // Update state
        if let Some(container) = self.containers.get_mut(id) {
            container.set_state(crate::container::ContainerState::Running);
        }

        // Persist to database
        if let Some(ref store) = self.store {
            store.update_container(id, crate::store::ContainerState::Running)?;
        }

        Ok(())
    }

    /// Stop a container
    pub fn stop_container(&mut self, id: &ContainerId) -> Result<(), StoreError> {
        // Get the jail name first
        let jail_name = self.containers.get(id)
            .ok_or_else(|| StoreError::SerializationError(format!("Container {} not found", id)))?
            .jail_name.clone();

        // Stop the jail
        self.stop_jail(&jail_name)
            .map_err(|e| StoreError::SerializationError(e.to_string()))?;

        // Update state
        if let Some(container) = self.containers.get_mut(id) {
            container.set_state(crate::container::ContainerState::Stopped);
        }

        // Persist to database
        if let Some(ref store) = self.store {
            store.update_container(id, crate::store::ContainerState::Stopped)?;
        }

        Ok(())
    }

    /// Remove a container
    pub fn remove_container(&mut self, id: &ContainerId) -> Result<(), StoreError> {
        let container = self.containers.remove(id)
            .ok_or_else(|| StoreError::SerializationError(format!("Container {} not found", id)))?;

        // Stop if running
        if container.is_running() {
            let _ = self.stop_jail(&container.jail_name);
        }

        // Destroy jail
        let _ = self.remove_jail(&container.jail_name);

        // Destroy ZFS dataset
        if let Some(ref zfs) = self.zfs {
            let _ = zfs.destroy(&container.dataset);
        }

        // Remove from database
        if let Some(ref store) = self.store {
            store.delete_container(id)?;
        }

        Ok(())
    }

    /// Get a container by ID
    pub fn get_container(&self, id: &ContainerId) -> Option<&Container> {
        self.containers.get(id).or_else(|| {
            // Try to load from database if not in memory
            if let Some(ref store) = self.store {
                if let Ok(Some(store_container)) = store.get_container(id) {
                    // Note: Same limitation as with images
                    return None;
                }
            }
            None
        })
    }

    /// List all containers
    pub fn list_containers(&self) -> Vec<&Container> {
        let mut containers: Vec<&Container> = self.containers.values().collect();

        // Also include containers from database
        if let Some(ref store) = self.store {
            if let Ok(db_containers) = store.list_containers() {
                debug!("Found {} containers in database (not yet loaded)", db_containers.len());
            }
        }

        containers
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
