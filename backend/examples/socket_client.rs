//! Example socket client for Kawakaze API
//!
//! This example demonstrates how to interact with the Kawakaze backend
//! using the JSON-over-Unix-socket API.
//!
//! Run with: sudo cargo run --example socket_client

use kawakaze_backend::api::{CreateJailRequest, Endpoint, Request};
use kawakaze_backend::server::JsonCodec;
use tokio::net::UnixStream;
use tokio_util::codec::Framed;
use futures::{SinkExt, StreamExt};
use serde_json::json;

const SOCKET_PATH: &str = "/var/run/kawakaze.sock";

/// Helper function to connect and send a request
async fn send_request(request: Request) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    // Connect to the socket
    let stream = UnixStream::connect(SOCKET_PATH).await?;

    // Create framed stream with JsonCodec for length-prefixed framing
    let mut framed = Framed::new(stream, JsonCodec);

    // Send the request as JSON
    let request_json = serde_json::to_value(&request)?;
    println!("Sending request: {}", serde_json::to_string_pretty(&request_json)?);

    framed.send(request_json).await?;

    // Receive the response
    let response_json = framed.next().await.ok_or("No response received")??;
    println!("Received response: {}", serde_json::to_string_pretty(&response_json)?);

    Ok(response_json)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Kawakaze Socket Client Example");
    println!("===============================\n");

    // Check if we're running as root
    #[cfg(unix)]
    if unsafe { libc::getuid() } != 0 {
        eprintln!("Warning: Not running as root. Some operations may fail.");
    }

    // 1. List all jails (should be empty initially)
    println!("1. Listing all jails...");
    let request = Request::get(Endpoint::Jails);
    let response = send_request(request).await?;
    println!("Status: {}\n", response["status"]);

    // 2. Create a new jail
    println!("2. Creating a new jail...");
    let create_req = CreateJailRequest {
        name: "example_jail".into(),
        path: Some("/tmp/example_jail".into()),
        ip: Some("192.168.1.100".into()),
    };
    let request = Request::post(Endpoint::Jails, create_req)?;
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if response["status"] == 201 {
        println!("Jail created successfully!");
    }
    println!();

    // 3. Get the jail info
    println!("3. Getting jail info...");
    let request = Request::get(Endpoint::Jail("example_jail".into()));
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    println!("Jail state: {}\n", response["data"]["state"]);

    // 4. List jails again (should show our jail)
    println!("4. Listing all jails...");
    let request = Request::get(Endpoint::Jails);
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if let Some(data) = response.get("data") {
        if let Some(jails) = data.as_array() {
            println!("Number of jails: {}", jails.len());
        }
    }
    println!();

    // 5. Try to start the jail (requires root)
    println!("5. Starting the jail...");
    let request = Request::post(
        Endpoint::StartJail("example_jail".into()),
        json!({})
    )?;
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if response["status"] == 200 {
        println!("Jail started successfully!");
    } else if response["status"] == 500 || response["status"] == 403 {
        println!("Note: Starting jail requires root privileges on FreeBSD");
    }
    println!();

    // 6. Get jail info again to see updated state
    println!("6. Getting updated jail info...");
    let request = Request::get(Endpoint::Jail("example_jail".into()));
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    println!("Jail state: {}\n", response["data"]["state"]);

    // 7. Try to stop the jail (if it was started)
    println!("7. Stopping the jail...");
    let request = Request::post(
        Endpoint::StopJail("example_jail".into()),
        json!({})
    )?;
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    println!();

    // 8. Create another jail with different configuration
    println!("8. Creating a second jail...");
    let create_req = CreateJailRequest {
        name: "webserver".into(),
        path: Some("/jails/webserver".into()),
        ip: None,
    };
    let request = Request::post(Endpoint::Jails, create_req)?;
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    println!();

    // 9. List all jails (should show 2 jails)
    println!("9. Listing all jails...");
    let request = Request::get(Endpoint::Jails);
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if let Some(data) = response.get("data") {
        if let Some(jails) = data.as_array() {
            println!("Number of jails: {}", jails.len());
            for jail in jails {
                println!("  - {} ({})", jail["name"], jail["state"]);
            }
        }
    }
    println!();

    // 10. Try to create a duplicate jail (should fail)
    println!("10. Trying to create duplicate jail...");
    let create_req = CreateJailRequest {
        name: "example_jail".into(), // Already exists
        path: None,
        ip: None,
    };
    let request = Request::post(Endpoint::Jails, create_req)?;
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if response["status"] == 409 {
        println!("Got expected conflict error: {}", response["error"]["message"]);
    }
    println!();

    // 11. Try to get nonexistent jail
    println!("11. Trying to get nonexistent jail...");
    let request = Request::get(Endpoint::Jail("nonexistent".into()));
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if response["status"] == 404 {
        println!("Got expected not found error: {}", response["error"]["message"]);
    }
    println!();

    // 12. Delete a jail
    println!("12. Deleting webserver jail...");
    let request = Request::delete(Endpoint::Jail("webserver".into()));
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    println!();

    // 13. List jails to verify deletion
    println!("13. Listing all jails after deletion...");
    let request = Request::get(Endpoint::Jails);
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if let Some(data) = response.get("data") {
        if let Some(jails) = data.as_array() {
            println!("Number of jails: {}", jails.len());
        }
    }
    println!();

    // 14. Try to delete already deleted jail
    println!("14. Trying to delete already deleted jail...");
    let request = Request::delete(Endpoint::Jail("webserver".into()));
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if response["status"] == 404 {
        println!("Got expected not found error: {}", response["error"]["message"]);
    }
    println!();

    // 15. Create jail with invalid name
    println!("15. Trying to create jail with invalid name...");
    let create_req = CreateJailRequest {
        name: "invalid name!".into(),
        path: None,
        ip: None,
    };
    let request = Request::post(Endpoint::Jails, create_req)?;
    let response = send_request(request).await?;
    println!("Status: {}", response["status"]);
    if response["status"] == 400 {
        println!("Got expected bad request error: {}", response["error"]["message"]);
    }
    println!();

    println!("Example completed!");

    Ok(())
}
