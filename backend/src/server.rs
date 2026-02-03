//! Unix socket server for Kawakaze API
//!
//! This module provides a JSON-over-Unix-socket server using length-prefixed framing.

use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tokio_util::codec::{Decoder, Encoder, Framed};
use futures::{SinkExt, StreamExt};
use bytes::Buf;
use tracing::{info, warn, error, debug, instrument};

use crate::api::Request;
use crate::handler::handle_request;
use crate::JailManager;

/// JSON codec for length-prefixed JSON messages
///
/// Message format: 4-byte big-endian length prefix + JSON payload
#[derive(Debug, Clone)]
pub struct JsonCodec;

impl Decoder for JsonCodec {
    type Item = serde_json::Value;
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Need at least 4 bytes for length prefix
        if src.len() < 4 {
            return Ok(None);
        }

        // Read length prefix (big-endian u32)
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(&src[0..4]);
        let len = u32::from_be_bytes(len_bytes) as usize;

        // Check if we have the full message
        if src.len() < 4 + len {
            return Ok(None);
        }

        // Remove length prefix and JSON payload from buffer
        src.advance(4);

        // Parse JSON payload
        let json_data = src.split_to(len);
        let value = serde_json::from_slice(&json_data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(Some(value))
    }
}

impl Encoder<serde_json::Value> for JsonCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: serde_json::Value, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        // Serialize JSON to bytes
        let json_bytes = serde_json::to_vec(&item)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        // Write length prefix (big-endian u32)
        let len = json_bytes.len() as u32;
        dst.extend_from_slice(&len.to_be_bytes());

        // Write JSON payload
        dst.extend_from_slice(&json_bytes);

        Ok(())
    }
}

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
    mut stream: tokio::net::UnixStream,
    manager: Arc<Mutex<JailManager>>,
    connection_id: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use Framed with our JsonCodec for length-prefixed messages
    let mut framed = Framed::new(&mut stream, JsonCodec);

    let mut request_count: u64 = 0;

    loop {
        // Read next JSON message
        match framed.next().await {
            Some(Ok(json_value)) => {
                request_count += 1;

                // Parse JSON as Request
                let request = match serde_json::from_value::<Request>(json_value) {
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
                        framed.send(error_response).await?;
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

                // Serialize response to JSON
                let response_json = match serde_json::to_value(&response) {
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
                        framed.send(error_json).await?;
                        continue;
                    }
                };

                // Send response
                framed.send(response_json).await?;
            }
            Some(Err(e)) => {
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
    use bytes::BytesMut;

    #[test]
    fn test_json_codec_encode() {
        let mut codec = JsonCodec;
        let mut dst = BytesMut::new();

        let json_value = serde_json::json!({"test": "data"});
        codec.encode(json_value.clone(), &mut dst).unwrap();

        // Should have 4-byte length prefix + JSON payload
        assert!(dst.len() > 4);

        // Verify length prefix
        let len_bytes = [dst[0], dst[1], dst[2], dst[3]];
        let len = u32::from_be_bytes(len_bytes) as usize;
        assert_eq!(len, dst.len() - 4);
    }

    #[test]
    fn test_json_codec_decode() {
        let mut codec = JsonCodec;
        let mut dst = BytesMut::new();

        let json_value = serde_json::json!({"test": "data"});
        codec.encode(json_value.clone(), &mut dst).unwrap();

        // Decode the message
        let decoded = codec.decode(&mut dst).unwrap().unwrap();
        assert_eq!(decoded, json_value);

        // Buffer should be empty after decoding
        assert!(dst.is_empty());
    }

    #[test]
    fn test_json_codec_partial_message() {
        let mut codec = JsonCodec;
        let mut dst = BytesMut::new();

        let json_value = serde_json::json!({"test": "data"});
        codec.encode(json_value, &mut dst).unwrap();

        // Take only part of the message
        let partial_len = dst.len() / 2;
        let mut partial = BytesMut::from(&dst[..partial_len]);
        dst.advance(partial_len);

        // Should return None for partial message
        assert!(codec.decode(&mut partial).unwrap().is_none());

        // Decode with the rest of the message
        partial.unsplit(dst);
        let decoded = codec.decode(&mut partial).unwrap().unwrap();
        assert!(decoded.is_object());
    }

    #[test]
    fn test_json_codec_empty_buffer() {
        let mut codec = JsonCodec;
        let mut dst = BytesMut::new();

        // Should return None for empty buffer
        assert!(codec.decode(&mut dst).unwrap().is_none());
    }

    #[test]
    fn test_json_codec_multiple_messages() {
        let mut codec = JsonCodec;
        let mut dst = BytesMut::new();

        // Encode multiple messages
        let msg1 = serde_json::json!({"msg": 1});
        let msg2 = serde_json::json!({"msg": 2});
        let msg3 = serde_json::json!({"msg": 3});

        codec.encode(msg1.clone(), &mut dst).unwrap();
        codec.encode(msg2.clone(), &mut dst).unwrap();
        codec.encode(msg3.clone(), &mut dst).unwrap();

        // Decode all messages
        let decoded1 = codec.decode(&mut dst).unwrap().unwrap();
        let decoded2 = codec.decode(&mut dst).unwrap().unwrap();
        let decoded3 = codec.decode(&mut dst).unwrap().unwrap();

        assert_eq!(decoded1, msg1);
        assert_eq!(decoded2, msg2);
        assert_eq!(decoded3, msg3);

        // Buffer should be empty
        assert!(codec.decode(&mut dst).unwrap().is_none());
    }
}
