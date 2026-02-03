//! Request handlers for the Kawakaze API
//!
//! This module contains handler functions that process API requests
//! and interact with the JailManager.

use std::sync::Arc;
use tokio::sync::Mutex;
use crate::api::{
    ApiError, BootstrapRequest, CreateJailRequest, Endpoint, JailInfo, JailListItem, Request, Response,
};
use crate::bootstrap::{Bootstrap, BootstrapConfig};
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
}
