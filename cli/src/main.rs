use clap::{Parser, Subcommand};
use futures::{SinkExt, StreamExt};
use kawakaze_backend::api::{
    BuildImageRequest, CreateContainerRequest, Endpoint, ExecRequest, PortMapping,
    Request,
};
use serde_json::Value;
use std::collections::HashMap;
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LinesCodec};

const SOCKET_PATH: &str = "/var/run/kawakaze.sock";

#[derive(Parser)]
#[command(name = "kawakaze")]
#[command(about = "Kawakaze - FreeBSD jail manager", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build an image from a Dockerfile
    Build {
        /// Path to the Dockerfile
        path: String,
        /// Name for the image
        #[arg(short, long)]
        name: String,
        /// Build arguments (key=value)
        #[arg(short, long)]
        build_args: Vec<String>,
    },

    /// Run a container
    Run {
        /// Image ID to run
        image: String,
        /// Container name
        #[arg(short, long)]
        name: Option<String>,
        /// Publish port (hostPort:containerPort or hostPort:containerPort/protocol)
        #[arg(short = 'p', long)]
        publish: Vec<String>,
        /// Volume mount (source:destination)
        #[arg(short = 'v', long)]
        volume: Vec<String>,
        /// Environment variable (key=value)
        #[arg(short, long)]
        env: Vec<String>,
        /// Restart policy (no, on-restart, on-fail)
        #[arg(long, default_value = "no")]
        restart: String,
        /// Working directory
        #[arg(long)]
        workdir: Option<String>,
        /// User to run as
        #[arg(long)]
        user: Option<String>,
        /// Command to run
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// List containers
    Ps,

    /// Start container
    Start {
        /// Container ID or name
        container: String,
    },

    /// Stop container
    Stop {
        /// Container ID or name
        container: String,
    },

    /// Remove container
    Rm {
        /// Container ID or name
        container: String,
        /// Force removal
        #[arg(short, long)]
        force: bool,
    },

    /// List images
    Images,

    /// Remove image
    Rmi {
        /// Image ID or name
        image: String,
        /// Force removal
        #[arg(short, long)]
        force: bool,
    },

    /// View container logs
    Logs {
        /// Container ID or name
        container: String,
        /// Follow log output
        #[arg(short, long)]
        follow: bool,
        /// Number of lines to show from the end
        #[arg(short = 'n', long, default_value = "100")]
        tail: usize,
    },

    /// Execute command in container
    Exec {
        /// Container ID or name
        container: String,
        /// Command to execute
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },

    /// Inspect image or container
    Inspect {
        /// Image or container ID
        id: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build {
            path,
            name,
            build_args,
        } => build_image(path, name, build_args).await,

        Commands::Run {
            image,
            name,
            publish,
            volume,
            env,
            restart,
            workdir: _,
            user: _,
            command,
        } => {
            run_container(image, name, publish, volume, env, restart, command).await
        }

        Commands::Ps => list_containers().await,

        Commands::Start { container } => start_container(container).await,

        Commands::Stop { container } => stop_container(container).await,

        Commands::Rm { container, force } => remove_container(container, force).await,

        Commands::Images => list_images().await,

        Commands::Rmi { image, force } => remove_image(image, force).await,

        Commands::Logs {
            container,
            follow,
            tail,
        } => container_logs(container, follow, tail).await,

        Commands::Exec { container, command } => exec_container(container, command).await,

        Commands::Inspect { id } => inspect(id).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

/// Connect to the Unix socket
async fn connect_to_socket() -> Result<Framed<UnixStream, LinesCodec>, String> {
    let stream = UnixStream::connect(SOCKET_PATH)
        .await
        .map_err(|e| format!("Failed to connect to backend: {}", e))?;

    Ok(Framed::new(stream, LinesCodec::new()))
}

/// Send a JSON request and get the response
async fn send_request(request: Request) -> Result<Value, String> {
    let mut socket = connect_to_socket().await?;

    // Convert Request to JSON string
    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    socket
        .send(request_json)
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    let response_line = socket
        .next()
        .await
        .ok_or("No response from backend")?
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let response: kawakaze_backend::api::Response = serde_json::from_str(&response_line)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if response.is_success() {
        Ok(response.data.unwrap_or(Value::Null))
    } else {
        let error = response.error.unwrap_or(kawakaze_backend::api::ApiError {
            code: "UNKNOWN".to_string(),
            message: "Unknown error".to_string(),
        });
        Err(format!("{}: {}", error.code, error.message))
    }
}

/// Format a JSON value for display
fn format_response(value: &Value) -> String {
    if value.is_null() {
        return String::new();
    }

    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

// ============================================================================
// Command Implementations
// ============================================================================

/// Build an image from a Dockerfile
async fn build_image(
    path: String,
    name: String,
    build_args: Vec<String>,
) -> Result<(), String> {
    // Read the Dockerfile
    let dockerfile_content =
        std::fs::read_to_string(&path).map_err(|e| format!("Failed to read Dockerfile: {}", e))?;

    // Parse build arguments
    let mut args_map = HashMap::new();
    for arg in build_args {
        let parts: Vec<&str> = arg.splitn(2, '=').collect();
        if parts.len() == 2 {
            args_map.insert(parts[0].to_string(), parts[1].to_string());
        }
    }

    let build_request = BuildImageRequest {
        name,
        dockerfile: dockerfile_content,
        build_args: args_map,
    };

    let request =
        Request::post(Endpoint::ImageBuild, build_request).map_err(|e| e.to_string())?;

    println!("Building image...");

    let response = send_request(request).await?;

    if let Some(image_id) = response.get("id") {
        println!("Built image: {}", image_id);
    } else {
        println!("Build complete");
    }

    Ok(())
}

/// Run a container
async fn run_container(
    image: String,
    name: Option<String>,
    publish: Vec<String>,
    volume: Vec<String>,
    env: Vec<String>,
    restart: String,
    command: Vec<String>,
) -> Result<(), String> {
    // Parse port mappings
    let ports: Vec<PortMapping> = publish
        .iter()
        .filter_map(|p| parse_port_mapping(p))
        .collect();

    // Parse volume mounts
    let volumes = volume
        .iter()
        .filter_map(|v| parse_volume_mount(v))
        .collect();

    // Parse environment variables
    let env_map: HashMap<String, String> = env
        .iter()
        .filter_map(|e| {
            let parts: Vec<&str> = e.splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect();

    let container_request = CreateContainerRequest {
        image_id: image,
        name,
        ports,
        volumes,
        env: env_map,
        restart_policy: restart,
        command: if command.is_empty() {
            None
        } else {
            Some(command)
        },
    };

    let request = Request::post(Endpoint::ContainerCreate, container_request)
        .map_err(|e| e.to_string())?;

    let response = send_request(request).await?;

    let container_id = response
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("No container ID in response")?;

    println!("Created container: {}", container_id);

    // Auto-start the container
    let start_request = Request::post(Endpoint::StartContainer(container_id.to_string()), ())
        .map_err(|e| e.to_string())?;

    send_request(start_request).await?;

    println!("Started container: {}", container_id);

    Ok(())
}

/// List all containers
async fn list_containers() -> Result<(), String> {
    let request = Request::get(Endpoint::Containers);
    let response = send_request(request).await?;

    if let Some(containers) = response.as_array() {
        if containers.is_empty() {
            println!("No containers found");
            return Ok(());
        }

        println!("{:<12} {:<20} {:<20} {:<10} {:<15}", "CONTAINER ID", "NAME", "IMAGE", "STATUS", "IP");

        for container in containers {
            let id = container.get("id").and_then(|v| v.as_str()).unwrap_or("N/A");
            let name = container.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let image = container.get("image_id").and_then(|v| v.as_str()).unwrap_or("N/A");
            let state = container.get("state").and_then(|v| v.as_str()).unwrap_or("unknown");
            let ip = container.get("ip").and_then(|v| v.as_str()).unwrap_or("");

            // Shorten IDs for display (first 12 chars)
            let short_id = if id.len() > 12 { &id[..12] } else { id };

            println!("{:<12} {:<20} {:<20} {:<10} {:<15}", short_id, name, image, state, ip);
        }
    } else {
        println!("No containers found");
    }

    Ok(())
}

/// Start a container
async fn start_container(container: String) -> Result<(), String> {
    let request = Request::post(Endpoint::StartContainer(container.clone()), ())
        .map_err(|e| e.to_string())?;

    println!("Starting container {}...", container);

    send_request(request).await?;

    println!("Container {} started", container);

    Ok(())
}

/// Stop a container
async fn stop_container(container: String) -> Result<(), String> {
    let request = Request::post(Endpoint::StopContainer(container.clone()), ())
        .map_err(|e| e.to_string())?;

    println!("Stopping container {}...", container);

    send_request(request).await?;

    println!("Container {} stopped", container);

    Ok(())
}

/// Remove a container
async fn remove_container(container: String, force: bool) -> Result<(), String> {
    if force {
        // Force stop first, then remove
        let _ = stop_container(container.clone()).await;
    }

    let request = Request::delete(Endpoint::RemoveContainer(container.clone()));

    println!("Removing container {}...", container);

    send_request(request).await?;

    println!("Container {} removed", container);

    Ok(())
}

/// List all images
async fn list_images() -> Result<(), String> {
    let request = Request::get(Endpoint::Images);
    let response = send_request(request).await?;

    if let Some(images) = response.as_array() {
        if images.is_empty() {
            println!("No images found");
            return Ok(());
        }

        println!("{:<12} {:<30} {:<15} {:<20}", "IMAGE ID", "NAME", "SIZE", "CREATED");

        for image in images {
            let id = image.get("id").and_then(|v| v.as_str()).unwrap_or("N/A");
            let name = image.get("name").and_then(|v| v.as_str()).unwrap_or("N/A");
            let size = image.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let created = image.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);

            // Shorten IDs for display
            let short_id = if id.len() > 12 { &id[..12] } else { id };

            // Format size
            let size_str = format_size(size);

            // Format timestamp (simple conversion)
            let created_str = if created > 0 {
                format_timestamp(created)
            } else {
                "unknown".to_string()
            };

            println!("{:<12} {:<30} {:<15} {:<20}", short_id, name, size_str, created_str);
        }
    } else {
        println!("No images found");
    }

    Ok(())
}

/// Remove an image
async fn remove_image(image: String, force: bool) -> Result<(), String> {
    let request = Request::delete(Endpoint::DeleteImage(image));

    if force {
        println!("Force removing image...");
    }

    println!("Removing image...");

    send_request(request).await?;

    println!("Image removed");

    Ok(())
}

/// View container logs
async fn container_logs(container: String, follow: bool, tail: usize) -> Result<(), String> {
    let mut socket = connect_to_socket().await?;

    let request = Request::get(Endpoint::ContainerLogs(container));
    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    socket
        .send(request_json)
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if follow {
        println!("Following logs (Ctrl+C to stop)...");
        while let Some(result) = socket.next().await {
            match result {
                Ok(line) => {
                    if let Ok(response) = serde_json::from_str::<kawakaze_backend::api::Response>(
                        &line,
                    ) {
                        if response.is_success() {
                            if let Some(data) = response.data {
                                if let Some(logs) = data.as_array() {
                                    for log in logs {
                                        if let Some(msg) = log.get("message").and_then(|v| v.as_str())
                                        {
                                            println!("{}", msg);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error reading logs: {}", e);
                    break;
                }
            }
        }
    } else {
        let response_line = socket
            .next()
            .await
            .ok_or("No response from backend")?
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let response: kawakaze_backend::api::Response =
            serde_json::from_str(&response_line).map_err(|e| format!("Failed to parse response: {}", e))?;

        if response.is_success() {
            if let Some(data) = response.data {
                if let Some(logs) = data.as_array() {
                    // Apply tail
                    let start = if logs.len() > tail { logs.len() - tail } else { 0 };
                    for log in logs.iter().skip(start) {
                        if let Some(msg) = log.get("message").and_then(|v| v.as_str()) {
                            println!("{}", msg);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Execute a command in a container
async fn exec_container(container: String, command: Vec<String>) -> Result<(), String> {
    if command.is_empty() {
        return Err("No command specified".to_string());
    }

    let exec_request = ExecRequest {
        command: command.clone(),
        env: HashMap::new(),
        workdir: None,
    };

    let request =
        Request::post(Endpoint::ContainerExec(container), exec_request).map_err(|e| e.to_string())?;

    println!("Executing: {}", command.join(" "));

    let response = send_request(request).await?;

    // Print output
    if let Some(stdout) = response.get("stdout").and_then(|v| v.as_str()) {
        print!("{}", stdout);
    }

    if let Some(stderr) = response.get("stderr").and_then(|v| v.as_str()) {
        eprint!("{}", stderr);
    }

    // Check exit code
    if let Some(exit_code) = response.get("exit_code").and_then(|v| v.as_i64()) {
        if exit_code != 0 {
            return Err(format!("Command exited with code {}", exit_code));
        }
    }

    Ok(())
}

/// Inspect an image or container
async fn inspect(id: String) -> Result<(), String> {
    // Try as container first, then image
    let request = Request::get(Endpoint::Container(id.clone()));

    let mut socket = connect_to_socket().await?;
    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    socket
        .send(request_json)
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    let response_line = socket
        .next()
        .await
        .ok_or("No response from backend")?
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let response: kawakaze_backend::api::Response =
        serde_json::from_str(&response_line).map_err(|e| format!("Failed to parse response: {}", e))?;

    if response.is_success() {
        if let Some(data) = response.data {
            println!("{}", format_response(&data));
        }
        return Ok(());
    }

    // Try as image
    let request = Request::get(Endpoint::Image(id.clone()));

    let mut socket = connect_to_socket().await?;
    let request_json = serde_json::to_string(&request)
        .map_err(|e| format!("Failed to serialize request: {}", e))?;

    socket
        .send(request_json)
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    let response_line = socket
        .next()
        .await
        .ok_or("No response from backend")?
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let response: kawakaze_backend::api::Response =
        serde_json::from_str(&response_line).map_err(|e| format!("Failed to parse response: {}", e))?;

    if response.is_success() {
        if let Some(data) = response.data {
            println!("{}", format_response(&data));
        }
        return Ok(());
    }

    Err(format!("No image or container found with ID: {}", id))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse a port mapping string (hostPort:containerPort or hostPort:containerPort/protocol)
fn parse_port_mapping(s: &str) -> Option<PortMapping> {
    let parts: Vec<&str> = s.split('/').collect();
    let protocol = if parts.len() > 1 { parts[1] } else { "tcp" };

    let port_parts: Vec<&str> = parts[0].split(':').collect();
    if port_parts.len() != 2 {
        return None;
    }

    let host_port: u16 = port_parts[0].parse().ok()?;
    let container_port: u16 = port_parts[1].parse().ok()?;

    Some(PortMapping {
        host_port,
        container_port,
        protocol: protocol.to_string(),
    })
}

/// Parse a volume mount string (source:destination)
fn parse_volume_mount(s: &str) -> Option<kawakaze_backend::api::Mount> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        return None;
    }

    Some(kawakaze_backend::api::Mount {
        source: parts[0].to_string(),
        destination: parts[1].to_string(),
        mount_type: "nullfs".to_string(), // Default to nullfs for now
    })
}

/// Format bytes to human-readable size
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Format Unix timestamp to human-readable date
fn format_timestamp(ts: i64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    if let Some(d) = UNIX_EPOCH.checked_add(Duration::from_secs(ts as u64)) {
        // Simple date formatting
        let datetime = chrono::DateTime::<chrono::Utc>::from(d);
        datetime.format("%Y-%m-%d %H:%M").to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_port_mapping() {
        let mapping = parse_port_mapping("8080:80").unwrap();
        assert_eq!(mapping.host_port, 8080);
        assert_eq!(mapping.container_port, 80);
        assert_eq!(mapping.protocol, "tcp");

        let mapping = parse_port_mapping("8080:80/udp").unwrap();
        assert_eq!(mapping.host_port, 8080);
        assert_eq!(mapping.container_port, 80);
        assert_eq!(mapping.protocol, "udp");
    }

    #[test]
    fn test_parse_volume_mount() {
        let mount = parse_volume_mount("/host/path:/container/path").unwrap();
        assert_eq!(mount.source, "/host/path");
        assert_eq!(mount.destination, "/container/path");
        assert_eq!(mount.mount_type, "nullfs");
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500B");
        assert_eq!(format_size(2048), "2.0KB");
        assert_eq!(format_size(5_242_880), "5.0MB");
        assert_eq!(format_size(1_073_741_824), "1.0GB");
    }
}
