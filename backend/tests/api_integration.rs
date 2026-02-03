//! Integration tests for the Kawakaze socket API
//!
//! These tests verify the end-to-end functionality of the API.

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::net::UnixStream;
use tokio_util::codec::Framed;

use kawakaze_backend::api::{CreateJailRequest, Endpoint, Method, Request};
use kawakaze_backend::server::JsonCodec;
use kawakaze_backend::JailManager;
use futures::{SinkExt, StreamExt};

/// Helper function to create a test jail manager
async fn create_test_manager(socket_path: &str) -> Arc<Mutex<JailManager>> {
    let manager = JailManager::new(socket_path);
    Arc::new(Mutex::new(manager))
}

/// Helper function to send a request and get a response
async fn send_request(
    socket_path: &str,
    request: Request,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    // Connect to socket
    let stream = UnixStream::connect(socket_path).await?;

    // Create framed stream with JsonCodec
    let mut framed = Framed::new(stream, JsonCodec);

    // Send request
    let request_json = serde_json::to_value(&request)?;
    framed.send(request_json).await?;

    // Receive response
    let response_json = framed.next().await.ok_or("No response")??;

    Ok(response_json)
}

/// Helper function to start the server in the background
async fn start_server(socket_path: &str) -> tokio::task::JoinHandle<()> {
    let manager = create_test_manager(socket_path).await;
    let server = kawakaze_backend::server::SocketServer::new(
        Arc::new(socket_path.to_string()),
        manager,
    );

    tokio::spawn(async move {
        let _ = server.run().await;
    })
}

#[tokio::test]
async fn test_server_starts_and_listens() {
    let socket_path = "/tmp/test_server.sock";

    // Start server
    let handle = start_server(socket_path).await;

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify socket exists
    assert!(std::path::Path::new(socket_path).exists());

    // Clean up
    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_list_jails_empty() {
    let socket_path = "/tmp/test_list_empty.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // List jails
    let request = Request::get(Endpoint::Jails);
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 200);
    assert!(response["data"].is_array());
    assert_eq!(response["data"].as_array().unwrap().len(), 0);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_create_and_list_jails() {
    let socket_path = "/tmp/test_create_list.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create a jail
    let create_req = CreateJailRequest {
        name: "test_jail".into(),
        path: Some("/tmp/test_jail_path".into()),
        ip: None,
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 201);
    assert_eq!(response["data"]["name"], "test_jail");

    // List jails
    let request = Request::get(Endpoint::Jails);
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 200);
    assert_eq!(response["data"].as_array().unwrap().len(), 1);
    assert_eq!(response["data"][0]["name"], "test_jail");

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_get_jail() {
    let socket_path = "/tmp/test_get_jail.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create a jail
    let create_req = CreateJailRequest {
        name: "my_jail".into(),
        path: None,
        ip: None,
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    send_request(socket_path, request).await.unwrap();

    // Get the jail
    let request = Request::get(Endpoint::Jail("my_jail".into()));
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 200);
    assert_eq!(response["data"]["name"], "my_jail");

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_get_nonexistent_jail() {
    let socket_path = "/tmp/test_get_nonexistent.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Try to get nonexistent jail
    let request = Request::get(Endpoint::Jail("nonexistent".into()));
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 404);
    assert!(response["error"].is_object());

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_create_invalid_jail_name() {
    let socket_path = "/tmp/test_invalid_name.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Try to create jail with invalid name
    let create_req = CreateJailRequest {
        name: "invalid name!".into(),
        path: None,
        ip: None,
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 400);
    assert!(response["error"].is_object());

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_create_duplicate_jail() {
    let socket_path = "/tmp/test_duplicate.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create jail
    let create_req = CreateJailRequest {
        name: "duplicate".into(),
        path: None,
        ip: None,
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req.clone()).unwrap();
    send_request(socket_path, request).await.unwrap();

    // Try to create again
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 409);
    assert!(response["error"].is_object());

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_delete_jail() {
    let socket_path = "/tmp/test_delete.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create a jail
    let create_req = CreateJailRequest {
        name: "to_delete".into(),
        path: None,
        ip: None,
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    send_request(socket_path, request).await.unwrap();

    // Delete the jail
    let request = Request::delete(Endpoint::Jail("to_delete".into()));
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 200);

    // Verify it's gone
    let request = Request::get(Endpoint::Jail("to_delete".into()));
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 404);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_delete_nonexistent_jail() {
    let socket_path = "/tmp/test_delete_nonexistent.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Try to delete nonexistent jail
    let request = Request::delete(Endpoint::Jail("nonexistent".into()));
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 404);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_concurrent_requests() {
    let socket_path = "/tmp/test_concurrent.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create multiple jails concurrently
    let mut tasks = Vec::new();
    for i in 0..5 {
        let socket_path = socket_path.to_string();
        let task = tokio::spawn(async move {
            let create_req = CreateJailRequest {
                name: format!("jail{}", i),
                path: None,
                ip: None,
                bootstrap: None,
            };
            let request = Request::post(Endpoint::Jails, &create_req).unwrap();
            send_request(&socket_path, request).await
        });
        tasks.push(task);
    }

    // Wait for all to complete
    for task in tasks {
        let result = task.await.unwrap();
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response["status"], 201);
    }

    // Verify all jails exist
    let request = Request::get(Endpoint::Jails);
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 200);
    assert_eq!(response["data"].as_array().unwrap().len(), 5);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_invalid_request_format() {
    let socket_path = "/tmp/test_invalid_request.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect and send invalid JSON
    let stream = UnixStream::connect(socket_path).await.unwrap();
    let mut framed = Framed::new(stream, JsonCodec);

    // Send invalid request (missing required fields)
    let invalid_json = serde_json::json!({"invalid": "request"});
    framed.send(invalid_json).await.unwrap();

    // Should receive error response
    let response = framed.next().await.unwrap().unwrap();
    assert_eq!(response["status"], 400);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_unknown_endpoint() {
    let socket_path = "/tmp/test_unknown_endpoint.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create request for unknown endpoint
    let request = Request {
        method: Method::Get,
        endpoint: "unknown/endpoint".to_string(),
        body: serde_json::Value::Null,
    };

    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 400);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_create_jail_with_path_and_ip() {
    let socket_path = "/tmp/test_jail_config.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create jail with path and IP
    let create_req = CreateJailRequest {
        name: "configured_jail".into(),
        path: Some("/jails/configured".into()),
        ip: Some("192.168.1.100".into()),
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 201);
    assert_eq!(response["data"]["name"], "configured_jail");
    assert_eq!(response["data"]["path"], "/jails/configured");

    // Get the jail to verify config
    let request = Request::get(Endpoint::Jail("configured_jail".into()));
    let response = send_request(socket_path, request).await.unwrap();

    assert_eq!(response["status"], 200);
    assert_eq!(response["data"]["path"], "/jails/configured");

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}

#[tokio::test]
async fn test_multiple_operations_same_jail() {
    let socket_path = "/tmp/test_multiple_ops.sock";

    let handle = start_server(socket_path).await;
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create jail
    let create_req = CreateJailRequest {
        name: "ops_jail".into(),
        path: None,
        ip: None,
        bootstrap: None,
    };
    let request = Request::post(Endpoint::Jails, create_req).unwrap();
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 201);

    // Get jail
    let request = Request::get(Endpoint::Jail("ops_jail".into()));
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 200);
    assert_eq!(response["data"]["state"], "created");

    // List jails (should include our jail)
    let request = Request::get(Endpoint::Jails);
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 200);
    assert_eq!(response["data"].as_array().unwrap().len(), 1);

    // Delete jail
    let request = Request::delete(Endpoint::Jail("ops_jail".into()));
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 200);

    // Verify deleted
    let request = Request::get(Endpoint::Jail("ops_jail".into()));
    let response = send_request(socket_path, request).await.unwrap();
    assert_eq!(response["status"], 404);

    handle.abort();
    let _ = std::fs::remove_file(socket_path);
}
