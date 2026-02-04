//! Unix socket server for Kawakaze API
//!
//! This module provides a JSON-over-Unix-socket server using line-delimited framing.

use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio_util::codec::{Framed, LinesCodec};
use futures::{SinkExt, StreamExt};
use tracing::{info, warn, error, debug, instrument};

use crate::api::Request;
use crate::handler::handle_request;
use crate::JailManager;

/// Unix socket server for the Kawakaze API
pub struct SocketServer {
    socket_path: Arc<String>,
    manager: Arc<Mutex<JailManager>>,
}

impl SocketServer {
    /// Create a new socket server
    pub fn new(socket_path: Arc<String>, manager: Arc<Mutex<JailManager>>) -> Self {
        Self {
            socket_path,
            manager,
        }
    }

    /// Run the socket server
    ///
    /// This method binds to the Unix socket and starts accepting connections.
    /// Each connection is handled in its own task.
    #[instrument(skip(self))]
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let socket_path = self.socket_path.as_ref();

        // Remove existing socket file if it exists
        if Path::new(socket_path).exists() {
            debug!("Removing existing socket file: {}", socket_path);
            std::fs::remove_file(socket_path)?;
        }

        // Bind to socket
        let listener = UnixListener::bind(socket_path)?;
        info!("Kawakaze API server listening on {}", socket_path);

        // Set appropriate permissions on the socket (read/write for owner only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(socket_path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(socket_path, perms)?;
            debug!("Set socket permissions to 0600");
        }

        let mut connection_count: u64 = 0;

        // Accept connections
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    connection_count += 1;
                    let manager = self.manager.clone();
                    let conn_id = connection_count;

                    debug!(connection_id = conn_id, peer_addr = ?addr, "New connection accepted");

                    // Spawn a new task for each connection
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, manager, conn_id).await {
                            error!(connection_id = conn_id, error = %e, "Connection error");
                        } else {
                            debug!(connection_id = conn_id, "Connection closed gracefully");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Accept error");
                }
            }
        }
    }
}

/// Handle a single client connection
#[instrument(skip(stream, manager), fields(connection_id = connection_id))]
async fn handle_connection(
    stream: tokio::net::UnixStream,
    manager: Arc<Mutex<JailManager>>,
    connection_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use Framed with LinesCodec for line-delimited JSON messages
    let mut framed = Framed::new(stream, LinesCodec::new());

    let mut request_count: u64 = 0;

    loop {
        // Read next JSON line
        match framed.next().await {
            Some(Ok(line)) => {
                request_count += 1;

                // Parse JSON as Request
                let request = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => req,
                    Err(e) => {
                        warn!(request_id = request_count, error = %e, "Invalid request format");
                        // Send error response for invalid request
                        let error_response = serde_json::json!({
                            "status": 400,
                            "error": {
                                "code": "INVALID_REQUEST",
                                "message": format!("Invalid request format: {}", e)
                            }
                        });
                        let response_line = serde_json::to_string(&error_response)
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
                        framed.send(response_line).await?;
                        continue;
                    }
                };

                // Log the incoming request
                info!(
                    request_id = request_count,
                    method = ?request.method,
                    endpoint = %request.endpoint,
                    "Incoming request"
                );

                // Handle the request
                let response = handle_request(request, manager.clone()).await;

                // Log the response status
                if response.is_success() {
                    debug!(
                        request_id = request_count,
                        status = response.status,
                        "Request successful"
                    );
                } else {
                    warn!(
                        request_id = request_count,
                        status = response.status,
                        error = response.error.as_ref().map(|e| e.message.as_str()),
                        "Request failed"
                    );
                }

                // Serialize response to JSON string
                let response_line = match serde_json::to_string(&response) {
                    Ok(json) => json,
                    Err(e) => {
                        error!(request_id = request_count, error = %e, "Failed to serialize response");
                        // This should rarely happen, but handle it gracefully
                        let error_json = serde_json::json!({
                            "status": 500,
                            "error": {
                                "code": "SERIALIZATION_ERROR",
                                "message": format!("Failed to serialize response: {}", e)
                            }
                        });
                        serde_json::to_string(&error_json)
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?
                    }
                };

                // Send response
                framed.send(response_line).await?;

                // For simple request/response model, close connection after one request
                break;
            }
            Some(Err(e)) => {
                // If we've already handled at least one request, the client might have
                // simply closed the connection after receiving the response
                if request_count > 0 {
                    debug!(connection_id = connection_id, "Client closed connection after request");
                    break;
                }
                error!(connection_id = connection_id, error = %e, "Frame decode error");
                return Err(Box::new(e) as Box<dyn std::error::Error>);
            }
            None => {
                // Connection closed
                debug!(connection_id = connection_id, total_requests = request_count, "Connection closed by client");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_server_creation() {
        let manager = Arc::new(Mutex::new(JailManager::new("/tmp/test.sock")));
        let server = SocketServer::new(Arc::new("/tmp/test.sock".to_string()), manager);
        assert_eq!(server.socket_path.as_ref(), "/tmp/test.sock");
    }
}
