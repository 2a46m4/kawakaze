//! Request handlers for the Kawakaze API
//!
//! This module contains handler functions that process API requests
//! and interact with the JailManager.

use std::sync::Arc;
use tokio::sync::Mutex;
use crate::api::{
    ApiError, BootstrapRequest, BuildImageRequest, ContainerInfo, ContainerListItem, CreateContainerRequest,
    CreateJailRequest, Endpoint, ExecRequest, ExecResult, ImageHistoryItem, ImageInfo, ImageListItem,
    JailInfo, JailListItem, Request, Response,
};
use crate::bootstrap::{Bootstrap, BootstrapConfig};
use crate::container::RestartPolicy;
use crate::image::Image;
use crate::image_builder::ImageBuildProgress;
use crate::JailManager;

/// Handle an API request and return a response
pub async fn handle_request(
    request: Request,
    manager: Arc<Mutex<JailManager>>,
) -> Response {
    // Parse the endpoint
    let endpoint = match request.parse_endpoint() {
        Ok(ep) => ep,
        Err(err) => return Response::bad_request(err.message),
    };

    // Route to appropriate handler based on endpoint and method
    match (&request.method, &endpoint) {
        // Jail endpoints
        (crate::api::Method::Get, Endpoint::Jails) => list_jails(manager).await,
        (crate::api::Method::Get, Endpoint::Jail(name)) => get_jail(manager, name).await,
        (crate::api::Method::Get, Endpoint::BootstrapStatus(name)) => get_bootstrap_progress(manager, name).await,
        (crate::api::Method::Post, Endpoint::Jails) => {
            match serde_json::from_value::<CreateJailRequest>(request.body) {
                Ok(create_req) => create_jail(manager, create_req).await,
                Err(err) => Response::bad_request(format!("Invalid request body: {}", err)),
            }
        }
        (crate::api::Method::Post, Endpoint::StartJail(name)) => start_jail(manager, name).await,
        (crate::api::Method::Post, Endpoint::StopJail(name)) => stop_jail(manager, name).await,
        (crate::api::Method::Post, Endpoint::BootstrapJail(name)) => {
            match serde_json::from_value::<BootstrapRequest>(request.body) {
                Ok(config) => bootstrap_jail(manager, name, config).await,
                Err(err) => Response::bad_request(format!("Invalid request body: {}", err)),
            }
        }
        (crate::api::Method::Delete, Endpoint::Jail(name)) => delete_jail(manager, name).await,

        // Image endpoints
        (crate::api::Method::Get, Endpoint::Images) => list_images(manager).await,
        (crate::api::Method::Get, Endpoint::Image(id_or_name)) => get_image(manager, id_or_name).await,
        (crate::api::Method::Post, Endpoint::ImageBuild) => {
            match serde_json::from_value::<BuildImageRequest>(request.body) {
                Ok(build_req) => build_image(manager, build_req).await,
                Err(err) => Response::bad_request(format!("Invalid request body: {}", err)),
            }
        }
        (crate::api::Method::Delete, Endpoint::DeleteImage(id_or_name)) => delete_image(manager, id_or_name).await,
        (crate::api::Method::Get, Endpoint::ImageHistory(id_or_name)) => get_image_history(manager, id_or_name).await,

        // Container endpoints
        (crate::api::Method::Get, Endpoint::Containers) => list_containers(manager).await,
        (crate::api::Method::Get, Endpoint::Container(id_or_name)) => get_container(manager, id_or_name).await,
        (crate::api::Method::Post, Endpoint::ContainerCreate) => {
            match serde_json::from_value::<CreateContainerRequest>(request.body) {
                Ok(create_req) => create_container(manager, create_req).await,
                Err(err) => Response::bad_request(format!("Invalid request body: {}", err)),
            }
        }
        (crate::api::Method::Post, Endpoint::StartContainer(id_or_name)) => start_container(manager, id_or_name).await,
        (crate::api::Method::Post, Endpoint::StopContainer(id_or_name)) => stop_container(manager, id_or_name).await,
        (crate::api::Method::Post, Endpoint::ContainerExec(id_or_name)) => {
            match serde_json::from_value::<ExecRequest>(request.body) {
                Ok(exec_req) => exec_container(manager, id_or_name, exec_req).await,
                Err(err) => Response::bad_request(format!("Invalid request body: {}", err)),
            }
        }
        (crate::api::Method::Delete, Endpoint::RemoveContainer(id_or_name)) => remove_container(manager, id_or_name, false).await,

        _ => Response::bad_request(format!(
            "Method {:?} not supported for endpoint {}",
            request.method, request.endpoint
        )),
    }
}

/// List all jails
async fn list_jails(manager: Arc<Mutex<JailManager>>) -> Response {
    let mgr: tokio::sync::MutexGuard<'_, JailManager> = manager.lock().await;
    let jail_names = mgr.jail_names();

    let items: Vec<JailListItem> = jail_names
        .into_iter()
        .map(|name| {
            if let Some(jail) = mgr.get_jail(&name) {
                JailListItem::from((name, jail.state()))
            } else {
                // This shouldn't happen, but handle it gracefully
                JailListItem::from((name, crate::jail::JailState::Created))
            }
        })
        .collect();

    match Response::success(items) {
        Ok(resp) => resp,
        Err(_) => Response::internal_error("Failed to serialize jail list"),
    }
}

/// Get information about a specific jail
async fn get_jail(manager: Arc<Mutex<JailManager>>, name: &str) -> Response {
    let mgr: tokio::sync::MutexGuard<'_, JailManager> = manager.lock().await;

    match mgr.get_jail(name) {
        Some(jail) => {
            let jail_info = JailInfo::from(jail.info());
            match Response::success(jail_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize jail info"),
            }
        }
        None => Response::not_found(format!("Jail '{}'", name)),
    }
}

/// Create a new jail
async fn create_jail(manager: Arc<Mutex<JailManager>>, request: CreateJailRequest) -> Response {
    // Validate the request
    if let Err(err) = request.validate() {
        return Response::bad_request(err.message);
    }

    let mut mgr: tokio::sync::MutexGuard<'_, JailManager> = manager.lock().await;

    // Check if jail already exists
    if mgr.get_jail(&request.name).is_some() {
        return Response::conflict(format!("Jail '{}' already exists", request.name));
    }

    // Create the jail
    let jail = match crate::jail::Jail::create(&request.name) {
        Ok(jail) => jail,
        Err(err) => {
            let api_err: ApiError = err.into();
            return match api_err.code.as_str() {
                "BAD_REQUEST" => Response::bad_request(api_err.message),
                "CONFLICT" => Response::conflict(api_err.message),
                _ => Response::internal_error(api_err.message),
            };
        }
    };

    // Apply optional parameters
    let mut jail = if let Some(ref path) = request.path {
        match jail.with_path(path) {
            Ok(j) => j,
            Err(err) => {
                let api_err: ApiError = err.into();
                return Response::bad_request(api_err.message);
            }
        }
    } else {
        jail
    };

    if let Some(ref ip) = request.ip {
        jail = match jail.with_ip(ip) {
            Ok(j) => j,
            Err(err) => {
                let api_err: ApiError = err.into();
                return Response::bad_request(api_err.message);
            }
        };
    }

    // Add to manager
    // Note: We need to add the jail to the manager's HashMap
    // This is a bit of a hack because add_jail creates a new jail
    // We should refactor JailManager to have an add_jail_with_config method
    // For now, we'll use the internal HashMap directly
    mgr.jails.insert(request.name.clone(), jail);

    let jail_info = JailInfo {
        name: request.name.clone(),
        jid: -1,
        state: "created".to_string(),
        path: request.path,
    };

    match Response::created(jail_info) {
        Ok(resp) => resp,
        Err(_) => Response::internal_error("Failed to serialize jail info"),
    }
}

/// Start a jail
async fn start_jail(manager: Arc<Mutex<JailManager>>, name: &str) -> Response {
    let mut mgr: tokio::sync::MutexGuard<'_, JailManager> = manager.lock().await;

    match mgr.start_jail(name) {
        Ok(()) => {
            let jail = mgr.get_jail(name).unwrap();
            let jail_info = JailInfo::from(jail.info());
            match Response::success(jail_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize jail info"),
            }
        }
        Err(err) => {
            let api_err: ApiError = err.into();
            match api_err.code.as_str() {
                "NOT_FOUND" => Response::not_found(api_err.message),
                "BAD_REQUEST" => Response::bad_request(api_err.message),
                _ => Response::internal_error(api_err.message),
            }
        }
    }
}

/// Stop a jail
async fn stop_jail(manager: Arc<Mutex<JailManager>>, name: &str) -> Response {
    let mut mgr: tokio::sync::MutexGuard<'_, JailManager> = manager.lock().await;

    match mgr.stop_jail(name) {
        Ok(()) => {
            let jail = mgr.get_jail(name).unwrap();
            let jail_info = JailInfo::from(jail.info());
            match Response::success(jail_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize jail info"),
            }
        }
        Err(err) => {
            let api_err: ApiError = err.into();
            match api_err.code.as_str() {
                "NOT_FOUND" => Response::not_found(api_err.message),
                "BAD_REQUEST" => Response::bad_request(api_err.message),
                _ => Response::internal_error(api_err.message),
            }
        }
    }
}

/// Delete a jail
async fn delete_jail(manager: Arc<Mutex<JailManager>>, name: &str) -> Response {
    let mut mgr: tokio::sync::MutexGuard<'_, JailManager> = manager.lock().await;

    match mgr.remove_jail(name) {
        Ok(()) => {
            match Response::success(serde_json::json!({"message": format!("Jail '{}' deleted", name)})) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize response"),
            }
        }
        Err(err) => {
            let api_err: ApiError = err.into();
            match api_err.code.as_str() {
                "NOT_FOUND" => Response::not_found(api_err.message),
                "BAD_REQUEST" => Response::bad_request(api_err.message),
                _ => Response::internal_error(api_err.message),
            }
        }
    }
}

/// Bootstrap a jail
async fn bootstrap_jail(
    manager: Arc<Mutex<JailManager>>,
    name: &str,
    config: BootstrapConfig,
) -> Response {
    // First check if jail exists
    let jail_path = {
        let mgr = manager.lock().await;
        match mgr.get_jail(name) {
            Some(jail) => {
                // Get the jail path
                match jail.info().path {
                    Some(ref p) => p.clone(),
                    None => {
                        // Use default path
                        format!("/tmp/{}", name)
                    }
                }
            }
            None => return Response::not_found(format!("Jail '{}'", name)),
        }
    };

    // Check if already bootstrapped
    if Bootstrap::is_bootstrapped(&jail_path) {
        return Response::conflict(format!(
            "Jail '{}' is already bootstrapped",
            name
        ));
    }

    // Start bootstrap in background
    let jail_name = name.to_string();
    let manager_clone = manager.clone();
    let jail_path_clone = jail_path.clone();

    tokio::spawn(async move {
        // Create progress channel
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(100);

        // Store the progress sender in the manager
        {
            let mut mgr = manager_clone.lock().await;
            mgr.register_bootstrap_tracker(jail_name.clone(), progress_tx.clone()).await;
        }

        // Spawn a task to forward progress updates to the manager
        let manager_for_progress = manager_clone.clone();
        let jail_name_for_progress = jail_name.clone();
        tokio::spawn(async move {
            while let Some(progress) = progress_rx.recv().await {
                let mut mgr = manager_for_progress.lock().await;
                // Update the stored progress
                if let Some(stored) = mgr.bootstrap_progress.get_mut(&jail_name_for_progress) {
                    *stored = progress.clone();
                }
            }
        });

        // Create and run bootstrap
        let bootstrap = match Bootstrap::new(&jail_path_clone, config, progress_tx) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("Failed to create bootstrap instance: {}", e);
                return;
            }
        };

        if let Err(e) = bootstrap.run().await {
            tracing::error!("Bootstrap failed for jail '{}': {}", jail_name, e);
        }
    });

    // Return immediately with 202 Accepted
    Response::error(202, ApiError::new("BOOTSTRAP_STARTED", format!("Bootstrap started for jail '{}'", name)))
}

/// Get bootstrap progress for a jail
async fn get_bootstrap_progress(manager: Arc<Mutex<JailManager>>, name: &str) -> Response {
    let mgr = manager.lock().await;

    match mgr.get_bootstrap_progress(name).await {
        Some(progress) => {
            match Response::success(progress) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize progress"),
            }
        }
        None => Response::not_found(format!("No bootstrap progress for jail '{}'", name)),
    }
}

// ============================================================================
// Image Handlers
// ============================================================================

/// List all images
async fn list_images(manager: Arc<Mutex<JailManager>>) -> Response {
    let mgr = manager.lock().await;
    let images = mgr.list_images();

    let items: Vec<ImageListItem> = images
        .into_iter()
        .map(|image| ImageListItem {
            id: image.id.clone(),
            name: image.name.clone(),
            size_bytes: image.size_bytes,
            created_at: image.created_at,
        })
        .collect();

    match Response::success(items) {
        Ok(resp) => resp,
        Err(_) => Response::internal_error("Failed to serialize image list"),
    }
}

/// Get image by ID or name
async fn get_image(manager: Arc<Mutex<JailManager>>, id_or_name: &str) -> Response {
    let mgr = manager.lock().await;

    // Try ID first, then name, then prefix
    let id_or_name_string = id_or_name.to_string();
    let image = mgr.get_image(&id_or_name_string)
        .or_else(|| mgr.get_image_by_name(id_or_name))
        .or_else(|| mgr.get_image_by_prefix(id_or_name));

    match image {
        Some(image) => {
            let image_info = ImageInfo {
                id: image.id.clone(),
                name: image.name.clone(),
                parent_id: image.parent_id.clone(),
                size_bytes: image.size_bytes,
                state: image.state.as_str().to_string(),
                created_at: image.created_at,
            };
            match Response::success(image_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize image info"),
            }
        }
        None => Response::not_found(format!("Image '{}'", id_or_name)),
    }
}

/// Build an image from a Dockerfile
async fn build_image(manager: Arc<Mutex<JailManager>>, request: BuildImageRequest) -> Response {
    // Validate request
    if request.name.is_empty() {
        return Response::bad_request("Image name cannot be empty");
    }

    if request.dockerfile.is_empty() {
        return Response::bad_request("Dockerfile cannot be empty");
    }

    let mut mgr = manager.lock().await;

    // Check if image with this name already exists
    if let Some(existing) = mgr.get_image_by_name(&request.name) {
        if existing.is_available() {
            return Response::conflict(format!("Image '{}' already exists", request.name));
        }
    }

    // Check if ZFS is available
    let zfs_pool_name = if mgr.zfs.is_some() {
        mgr.config.zfs_pool.clone()
    } else {
        return Response::internal_error("ZFS not configured");
    };

    let base_dataset = format!("{}/images", zfs_pool_name);

    // Store build args for background task
    let build_args = request.build_args.clone();

    // Create ImageBuilder - note: we need to recreate Zfs in the background task
    let _base_dataset_clone = base_dataset.clone();

    // Parse dockerfile to get FROM image
    let from_image = match parse_from_instruction(&request.dockerfile) {
        Ok(from_name) => {
            // Handle "scratch" as a special case - no base image
            if from_name == "scratch" {
                None
            } else if let Some(img) = mgr.get_image_by_name(&from_name) {
                Some(img.clone())
            } else {
                return Response::bad_request(format!(
                    "Base image '{}' not found. Ensure the base image exists or build it first.",
                    from_name
                ));
            }
        }
        Err(_) => None, // No FROM instruction
    };

    // Generate image ID
    let image_id = Image::generate_id();
    let image_id_clone = image_id.clone();
    let name_clone = request.name.clone();
    let dockerfile_clone = request.dockerfile.clone();
    let from_image_clone = from_image.clone();
    let build_args_clone = build_args.clone();

    // Create progress channel
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(100);

    // Register progress tracker
    mgr.image_build_tracker.insert(image_id.clone(), progress_tx.clone());
    mgr.image_build_progress.insert(
        image_id.clone(),
        ImageBuildProgress {
            image_id: image_id.clone(),
            step: 0,
            total_steps: 0,
            current_instruction: "Initializing...".to_string(),
            status: crate::image_builder::BuildStatus::Building,
        },
    );

    // Clone manager for background task
    let manager_clone = manager.clone();

    // Spawn background build task
    tokio::spawn(async move {
        // Create a new builder for the background task
        let mgr_inner = manager_clone.lock().await;
        let zfs_inner = match mgr_inner.zfs.as_ref() {
            Some(_z) => {
                // Create a new Zfs instance with the same pool
                match crate::zfs::Zfs::new(&mgr_inner.config.zfs_pool) {
                    Ok(z) => z,
                    Err(_) => {
                        let _ = progress_tx
                            .send(ImageBuildProgress {
                                image_id: image_id_clone.clone(),
                                step: 0,
                                total_steps: 0,
                                current_instruction: "Failed to create ZFS instance".to_string(),
                                status: crate::image_builder::BuildStatus::Failed,
                            })
                            .await;
                        return;
                    }
                }
            }
            None => {
                let _ = progress_tx
                    .send(ImageBuildProgress {
                        image_id: image_id_clone.clone(),
                        step: 0,
                        total_steps: 0,
                        current_instruction: "ZFS not configured".to_string(),
                        status: crate::image_builder::BuildStatus::Failed,
                    })
                    .await;
                return;
            }
        };

        let base_dataset_inner = format!("{}/images", mgr_inner.config.zfs_pool);
        drop(mgr_inner);

        let (mut builder_inner, _rx) =
            crate::image_builder::ImageBuilder::new(zfs_inner, base_dataset_inner);

        // Set build args if provided
        if !build_args_clone.is_empty() {
            builder_inner = builder_inner.with_build_args(build_args_clone);
        }

        let result = builder_inner
            .build(name_clone.clone(), &dockerfile_clone, from_image_clone.as_ref())
            .await;

        match result {
            Ok(image) => {
                // Store image in manager
                let mut mgr_inner = manager_clone.lock().await;
                if let Err(e) = mgr_inner.add_image(image.clone()) {
                    tracing::error!("Failed to store image in manager: {}", e);
                }

                // Update progress to complete
                mgr_inner.image_build_progress.insert(
                    image_id_clone.clone(),
                    ImageBuildProgress {
                        image_id: image_id_clone.clone(),
                        step: image.dockerfile.len(),
                        total_steps: image.dockerfile.len(),
                        current_instruction: "Build complete".to_string(),
                        status: crate::image_builder::BuildStatus::Complete,
                    },
                );
            }
            Err(e) => {
                tracing::error!("Image build failed: {}", e);

                // Update progress to failed
                let mut mgr_inner = manager_clone.lock().await;
                mgr_inner.image_build_progress.insert(
                    image_id_clone.clone(),
                    ImageBuildProgress {
                        image_id: image_id_clone.clone(),
                        step: 0,
                        total_steps: 0,
                        current_instruction: format!("Build failed: {}", e),
                        status: crate::image_builder::BuildStatus::Failed,
                    },
                );
            }
        }
    });

    // Spawn a task to forward progress updates to the manager
    let manager_for_progress = manager.clone();
    let image_id_for_progress = image_id.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let mut mgr = manager_for_progress.lock().await;
            // Update the stored progress
            if let Some(stored) = mgr.image_build_progress.get_mut(&image_id_for_progress) {
                *stored = progress.clone();
            }
        }
    });

    // Return immediately with 202 Accepted
    Response::error(
        202,
        ApiError::new(
            "BUILD_STARTED",
            format!(
                "Image build started for '{}'. ID: {}. Use GET /images/{}/history to track progress.",
                request.name, image_id, image_id
            ),
        ),
    )
}

/// Delete an image
async fn delete_image(manager: Arc<Mutex<JailManager>>, id_or_name: &str) -> Response {
    let mut mgr = manager.lock().await;

    // Try to find the image (exact ID, name, or prefix)
    let id_or_name_string = id_or_name.to_string();
    let image_id = if let Some(image) = mgr.get_image(&id_or_name_string) {
        image.id.clone()
    } else if let Some(image) = mgr.get_image_by_name(id_or_name) {
        image.id.clone()
    } else if let Some(image) = mgr.get_image_by_prefix(id_or_name) {
        image.id.clone()
    } else {
        return Response::not_found(format!("Image '{}'", id_or_name));
    };

    match mgr.remove_image(&image_id) {
        Ok(()) => {
            match Response::success(serde_json::json!({"message": format!("Image '{}' deleted", id_or_name)})) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize response"),
            }
        }
        Err(e) => Response::internal_error(format!("Failed to delete image: {}", e)),
    }
}

/// Get image history
async fn get_image_history(manager: Arc<Mutex<JailManager>>, id_or_name: &str) -> Response {
    let mgr = manager.lock().await;

    // Try to find the image (exact ID, name, or prefix)
    let id_or_name_string = id_or_name.to_string();
    let image = mgr.get_image(&id_or_name_string)
        .or_else(|| mgr.get_image_by_name(id_or_name))
        .or_else(|| mgr.get_image_by_prefix(id_or_name));

    match image {
        Some(img) => {
            // Convert Dockerfile instructions to history items
            let history: Vec<ImageHistoryItem> = img
                .dockerfile
                .iter()
                .enumerate()
                .map(|(idx, instr)| {
                    let created_by = match instr {
                        crate::image::DockerfileInstruction::From(img) => format!("FROM {}", img),
                        crate::image::DockerfileInstruction::Run(cmd) => format!("RUN {}", cmd),
                        crate::image::DockerfileInstruction::Copy { src, dest, .. } => {
                            format!("COPY {} {}", src, dest)
                        }
                        crate::image::DockerfileInstruction::Add { src, dest } => {
                            format!("ADD {} {}", src, dest)
                        }
                        crate::image::DockerfileInstruction::WorkDir(path) => {
                            format!("WORKDIR {}", path)
                        }
                        crate::image::DockerfileInstruction::Env(env) => {
                            format!("ENV {} vars", env.len())
                        }
                        crate::image::DockerfileInstruction::Expose(ports) => {
                            format!("EXPOSE {:?}", ports)
                        }
                        crate::image::DockerfileInstruction::User(user) => format!("USER {}", user),
                        crate::image::DockerfileInstruction::Volume(vols) => {
                            format!("VOLUME {:?}", vols)
                        }
                        crate::image::DockerfileInstruction::Cmd(cmd) => format!("CMD {:?}", cmd),
                        crate::image::DockerfileInstruction::Entrypoint(ep) => {
                            format!("ENTRYPOINT {:?}", ep)
                        }
                        crate::image::DockerfileInstruction::Label(labels) => {
                            format!("LABEL {} entries", labels.len())
                        }
                    };

                    // Estimate layer size (in reality, each layer would have its own size)
                    let layer_size = if !img.dockerfile.is_empty() {
                        img.size_bytes / img.dockerfile.len() as u64
                    } else {
                        0
                    };

                    ImageHistoryItem {
                        id: format!("{}-layer-{}", img.id, idx),
                        created_at: img.created_at, // In reality, each layer would have its own timestamp
                        size_bytes: layer_size,
                        created_by,
                    }
                })
                .collect();

            match Response::success(history) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize image history"),
            }
        }
        None => Response::not_found(format!("Image '{}'", id_or_name)),
    }
}

/// Helper: Parse the FROM instruction from a Dockerfile to get the base image name
fn parse_from_instruction(dockerfile: &str) -> Result<String, &'static str> {
    for line in dockerfile.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line_upper = line.to_uppercase();
        if line_upper.starts_with("FROM ") {
            let parts = line[5..].trim().split_whitespace().collect::<Vec<_>>();
            if !parts.is_empty() {
                return Ok(parts[0].to_string());
            }
        }

        // Stop after first non-FROM instruction
        if !line_upper.starts_with("FROM") && !line_upper.starts_with('#') {
            break;
        }
    }

    Err("No FROM instruction found")
}

// ============================================================================
// Container Handlers
// ============================================================================

/// List all containers
async fn list_containers(manager: Arc<Mutex<JailManager>>) -> Response {
    let mgr = manager.lock().await;
    let containers = mgr.list_containers();

    let items: Vec<ContainerListItem> = containers
        .iter()
        .map(|c| ContainerListItem {
            id: c.id.clone(),
            name: c.name.clone(),
            image_id: c.image_id.clone(),
            state: c.state.as_str().to_string(),
            ip: c.ip.clone(),
        })
        .collect();

    match Response::success(items) {
        Ok(resp) => resp,
        Err(_) => Response::internal_error("Failed to serialize container list"),
    }
}

/// Get container by ID, name, or prefix
async fn get_container(manager: Arc<Mutex<JailManager>>, id_or_name: &str) -> Response {
    let mgr = manager.lock().await;

    // Try ID first, then prefix, then search by name
    let id_or_name_string = id_or_name.to_string();
    let container = mgr.get_container(&id_or_name_string)
        .or_else(|| mgr.get_container_by_prefix(id_or_name))
        .or_else(|| {
            mgr.list_containers()
                .into_iter()
                .find(|c| c.name.as_deref() == Some(id_or_name))
        });

    match container {
        Some(container) => {
            let container_info = ContainerInfo {
                id: container.id.clone(),
                name: container.name.clone(),
                image_id: container.image_id.clone(),
                state: container.state.as_str().to_string(),
                ip: container.ip.clone(),
                restart_policy: container.restart_policy.as_str().to_string(),
                created_at: container.created_at,
                started_at: container.started_at,
            };
            match Response::success(container_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize container info"),
            }
        }
        None => Response::not_found(format!("Container '{}'", id_or_name)),
    }
}

/// Create container from image
async fn create_container(manager: Arc<Mutex<JailManager>>, request: CreateContainerRequest) -> Response {
    let mut mgr = manager.lock().await;

    // Validate image exists (try exact ID, then name, then prefix)
    let image = mgr.get_image(&request.image_id)
        .or_else(|| mgr.get_image_by_name(&request.image_id))
        .or_else(|| mgr.get_image_by_prefix(&request.image_id));

    if image.is_none() {
        return Response::not_found(format!("Image '{}'", request.image_id));
    }

    // Parse restart policy
    let restart_policy = match request.restart_policy.parse::<RestartPolicy>() {
        Ok(policy) => policy,
        Err(_) => {
            return Response::bad_request(format!("Invalid restart policy: {}", request.restart_policy));
        }
    };

    // Convert API port mappings to internal format
    let port_mappings: Vec<crate::container::PortMapping> = request.ports
        .into_iter()
        .map(|p| {
            let protocol = match p.protocol.as_str() {
                "tcp" => crate::container::PortProtocol::Tcp,
                "udp" => crate::container::PortProtocol::Udp,
                _ => crate::container::PortProtocol::Tcp,
            };
            crate::container::PortMapping::new(p.host_port, p.container_port, protocol)
        })
        .collect();

    // Convert API mounts to internal format
    let mounts: Vec<crate::container::Mount> = request.volumes
        .into_iter()
        .map(|v| {
            let mount_type = match v.mount_type.as_str() {
                "zfs" => crate::container::MountType::Zfs,
                "nullfs" => crate::container::MountType::Nullfs,
                _ => crate::container::MountType::Nullfs,
            };
            crate::container::Mount::new(v.source, v.destination, mount_type, false)
        })
        .collect();

    // Create container config - use the resolved full image ID
    let config = crate::container::ContainerConfig {
        image_id: image.unwrap().id.clone(),
        name: request.name.clone(),
        ports: port_mappings,
        volumes: mounts,
        restart_policy,
    };

    match mgr.create_container(config) {
        Ok(container) => {
            let container_info = ContainerInfo {
                id: container.id.clone(),
                name: container.name.clone(),
                image_id: container.image_id.clone(),
                state: container.state.as_str().to_string(),
                ip: container.ip.clone(),
                restart_policy: container.restart_policy.as_str().to_string(),
                created_at: container.created_at,
                started_at: container.started_at,
            };
            match Response::created(container_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize container info"),
            }
        }
        Err(e) => Response::internal_error(format!("Failed to create container: {}", e)),
    }
}

/// Start container
async fn start_container(manager: Arc<Mutex<JailManager>>, id_or_name: &str) -> Response {
    let mut mgr = manager.lock().await;

    // Find container by ID, name, or prefix
    let id_or_name_string = id_or_name.to_string();
    let container_id = if let Some(c) = mgr.get_container(&id_or_name_string) {
        c.id.clone()
    } else if let Some(c) = mgr.get_container_by_prefix(id_or_name) {
        c.id.clone()
    } else {
        match mgr.list_containers()
            .into_iter()
            .find(|c| c.name.as_deref() == Some(id_or_name))
        {
            Some(c) => c.id.clone(),
            None => return Response::not_found(format!("Container '{}'", id_or_name)),
        }
    };

    match mgr.start_container(&container_id) {
        Ok(()) => {
            let container = mgr.get_container(&container_id).unwrap();
            let container_info = ContainerInfo {
                id: container.id.clone(),
                name: container.name.clone(),
                image_id: container.image_id.clone(),
                state: container.state.as_str().to_string(),
                ip: container.ip.clone(),
                restart_policy: container.restart_policy.as_str().to_string(),
                created_at: container.created_at,
                started_at: container.started_at,
            };
            match Response::success(container_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize container info"),
            }
        }
        Err(e) => Response::internal_error(format!("Failed to start container: {}", e)),
    }
}

/// Stop container
async fn stop_container(manager: Arc<Mutex<JailManager>>, id_or_name: &str) -> Response {
    let mut mgr = manager.lock().await;

    // Find container by ID, name, or prefix
    let id_or_name_string = id_or_name.to_string();
    let container_id = if let Some(c) = mgr.get_container(&id_or_name_string) {
        c.id.clone()
    } else if let Some(c) = mgr.get_container_by_prefix(id_or_name) {
        c.id.clone()
    } else {
        match mgr.list_containers()
            .into_iter()
            .find(|c| c.name.as_deref() == Some(id_or_name))
        {
            Some(c) => c.id.clone(),
            None => return Response::not_found(format!("Container '{}'", id_or_name)),
        }
    };

    match mgr.stop_container(&container_id) {
        Ok(()) => {
            let container = mgr.get_container(&container_id).unwrap();
            let container_info = ContainerInfo {
                id: container.id.clone(),
                name: container.name.clone(),
                image_id: container.image_id.clone(),
                state: container.state.as_str().to_string(),
                ip: container.ip.clone(),
                restart_policy: container.restart_policy.as_str().to_string(),
                created_at: container.created_at,
                started_at: container.started_at,
            };
            match Response::success(container_info) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize container info"),
            }
        }
        Err(e) => Response::internal_error(format!("Failed to stop container: {}", e)),
    }
}

/// Remove container
async fn remove_container(manager: Arc<Mutex<JailManager>>, id_or_name: &str, force: bool) -> Response {
    let mut mgr = manager.lock().await;

    // Find container by ID, name, or prefix
    let id_or_name_string = id_or_name.to_string();
    let container_id = if let Some(c) = mgr.get_container(&id_or_name_string) {
        c.id.clone()
    } else if let Some(c) = mgr.get_container_by_prefix(id_or_name) {
        c.id.clone()
    } else {
        match mgr.list_containers()
            .into_iter()
            .find(|c| c.name.as_deref() == Some(id_or_name))
        {
            Some(c) => c.id.clone(),
            None => return Response::not_found(format!("Container '{}'", id_or_name)),
        }
    };

    // Check if container is running
    if let Some(container) = mgr.get_container(&container_id) {
        if container.is_running() && !force {
            return Response::bad_request(format!(
                "Container '{}' is running. Stop it first or use force flag.",
                id_or_name
            ));
        }
    }

    match mgr.remove_container(&container_id) {
        Ok(()) => {
            match Response::success(serde_json::json!({"message": format!("Container '{}' removed", id_or_name)})) {
                Ok(resp) => resp,
                Err(_) => Response::internal_error("Failed to serialize response"),
            }
        }
        Err(e) => Response::internal_error(format!("Failed to remove container: {}", e)),
    }
}

/// Execute command in container
async fn exec_container(manager: Arc<Mutex<JailManager>>, id_or_name: &str, exec_req: ExecRequest) -> Response {
    let mgr = manager.lock().await;

    // Find container by ID, name, or prefix
    let id_or_name_string = id_or_name.to_string();
    let container = mgr.get_container(&id_or_name_string)
        .or_else(|| mgr.get_container_by_prefix(id_or_name))
        .or_else(|| {
            mgr.list_containers()
                .into_iter()
                .find(|c| c.name.as_deref() == Some(id_or_name))
        });

    let container = match container {
        Some(c) => c,
        None => return Response::not_found(format!("Container '{}'", id_or_name)),
    };

    // Check if container is running
    if !container.is_running() {
        return Response::bad_request(format!(
            "Container '{}' is not running. Start it first.",
            id_or_name
        ));
    }

    // Build the command
    let mut cmd_args = Vec::new();
    if let Some(ref workdir) = exec_req.workdir {
        cmd_args.extend(["-U".to_string(), "root".to_string()]);
        cmd_args.extend(["-c".to_string(), format!("cd {}", workdir)]);
    }
    cmd_args.extend(container.jail_name.clone().into_bytes().iter().map(|b| *b as char).collect::<String>().lines().map(String::from));

    // For simplicity, we'll use jexec to run the command
    // jexec [-l -u username] jail command [args ...]
    let output = match std::process::Command::new("jexec")
        .arg(&container.jail_name)
        .args(&exec_req.command)
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return Response::internal_error(format!("Failed to execute command: {}", e));
        }
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let result = ExecResult {
        exit_code,
        stdout,
        stderr,
    };

    match Response::success(result) {
        Ok(resp) => resp,
        Err(_) => Response::internal_error("Failed to serialize exec result"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Method, status};

    fn create_test_manager() -> JailManager {
        JailManager::new("/tmp/test-handler.sock")
    }

    #[tokio::test]
    async fn test_handle_request_list_jails() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        // Add some jails
        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("jail1").unwrap();
            mgr.add_jail("jail2").unwrap();
        }

        let request = Request::get(crate::api::Endpoint::Jails);
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::OK);
        assert!(response.is_success());
        assert!(response.data.is_some());
    }

    #[tokio::test]
    async fn test_handle_request_get_jail() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("test_jail").unwrap();
        }

        let request = Request::get(crate::api::Endpoint::Jail("test_jail".into()));
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::OK);
        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_handle_request_get_jail_not_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let request = Request::get(crate::api::Endpoint::Jail("nonexistent".into()));
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::NOT_FOUND);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_handle_request_create_jail() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let create_req = CreateJailRequest {
            name: "new_jail".into(),
            path: Some("/tmp/new_jail".into()),
            ip: Some("192.168.1.100".into()),
            bootstrap: None,
        };

        let request = Request::post(crate::api::Endpoint::Jails, create_req).unwrap();
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::CREATED);
        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_handle_request_create_jail_invalid_name() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let create_req = CreateJailRequest {
            name: "invalid name!".into(),
            path: None,
            ip: None,
            bootstrap: None,
        };

        let request = Request::post(crate::api::Endpoint::Jails, create_req).unwrap();
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::BAD_REQUEST);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_handle_request_create_jail_duplicate() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("existing_jail").unwrap();
        }

        let create_req = CreateJailRequest {
            name: "existing_jail".into(),
            path: None,
            ip: None,
            bootstrap: None,
        };

        let request = Request::post(crate::api::Endpoint::Jails, create_req).unwrap();
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::CONFLICT);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_handle_request_invalid_endpoint() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let request = Request {
            method: Method::Get,
            endpoint: "invalid/endpoint".to_string(),
            body: serde_json::Value::Null,
        };

        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_request_unsupported_method() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        // DELETE on /jails is not supported
        let request = Request::delete(crate::api::Endpoint::Jails);
        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_list_jails_empty() {
        let manager = Arc::new(Mutex::new(create_test_manager()));
        let response = list_jails(manager).await;

        assert_eq!(response.status, status::OK);
        assert!(response.is_success());

        // Verify empty list
        let data = response.data.unwrap();
        let items: Vec<JailListItem> = serde_json::from_value(data).unwrap();
        assert_eq!(items.len(), 0);
    }

    #[tokio::test]
    async fn test_get_jail_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("test_jail").unwrap();
        }

        let response = get_jail(manager, "test_jail").await;

        assert_eq!(response.status, status::OK);
        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_get_jail_not_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));
        let response = get_jail(manager, "nonexistent").await;

        assert_eq!(response.status, status::NOT_FOUND);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_create_jail_success() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let request = CreateJailRequest {
            name: "new_jail".into(),
            path: None,
            ip: None,
            bootstrap: None,
        };

        let response = create_jail(manager, request).await;

        assert_eq!(response.status, status::CREATED);
        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_create_jail_invalid_request() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let request = CreateJailRequest {
            name: "".into(),
            path: None,
            ip: None,
            bootstrap: None,
        };

        let response = create_jail(manager, request).await;

        assert_eq!(response.status, status::BAD_REQUEST);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_start_jail_success() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let manager = Arc::new(Mutex::new(create_test_manager()));

        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("test_jail").unwrap();
        }

        let response = start_jail(manager, "test_jail").await;

        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_start_jail_not_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));
        let response = start_jail(manager, "nonexistent").await;

        assert_eq!(response.status, status::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_stop_jail_success() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let manager = Arc::new(Mutex::new(create_test_manager()));

        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("test_jail").unwrap();
            mgr.start_jail("test_jail").unwrap();
        }

        let response = stop_jail(manager, "test_jail").await;

        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_stop_jail_not_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));
        let response = stop_jail(manager, "nonexistent").await;

        assert_eq!(response.status, status::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_delete_jail_success() {
        // Skip if not running as root
        #[cfg(unix)]
        if unsafe { libc::getuid() } != 0 {
            return;
        }

        let manager = Arc::new(Mutex::new(create_test_manager()));

        {
            let mut mgr = manager.lock().await;
            mgr.add_jail("test_jail").unwrap();
        }

        let response = delete_jail(manager, "test_jail").await;

        assert!(response.is_success());
    }

    #[tokio::test]
    async fn test_delete_jail_not_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));
        let response = delete_jail(manager, "nonexistent").await;

        assert_eq!(response.status, status::NOT_FOUND);
    }

    // ============================================================================
    // Exec Container Tests
    // ============================================================================

    #[tokio::test]
    async fn test_exec_container_not_found() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let exec_req = ExecRequest {
            command: vec!["echo".to_string(), "test".to_string()],
            env: std::collections::HashMap::new(),
            workdir: None,
        };

        let response = exec_container(manager, "nonexistent", exec_req).await;

        assert_eq!(response.status, status::NOT_FOUND);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_exec_container_invalid_request_body() {
        let manager = Arc::new(Mutex::new(create_test_manager()));

        let request = Request::post(
            crate::api::Endpoint::ContainerExec("some-id".into()),
            "invalid json",
        ).unwrap();

        let response = handle_request(request, manager).await;

        assert_eq!(response.status, status::BAD_REQUEST);
        assert!(!response.is_success());
    }

    #[tokio::test]
    async fn test_exec_request_valid() {
        let exec_req = ExecRequest {
            command: vec!["ls".to_string(), "-la".to_string()],
            env: {
                let mut map = std::collections::HashMap::new();
                map.insert("PATH".to_string(), "/usr/bin".to_string());
                map
            },
            workdir: Some("/tmp".to_string()),
        };

        assert_eq!(exec_req.command.len(), 2);
        assert_eq!(exec_req.workdir, Some("/tmp".to_string()));
        assert_eq!(exec_req.env.len(), 1);
    }
}
