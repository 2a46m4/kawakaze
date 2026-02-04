//! Kawakaze backend binary
//!
//! This is the main entry point for running the Kawakaze jail manager backend.

use kawakaze_backend::{JailManager, config::KawakazeConfig};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber
    // Set RUST_LOG environment variable to control logging (e.g., RUST_LOG=debug)
    let env_filter = EnvFilter::from_default_env()
        .add_directive(tracing::Level::INFO.into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .init();

    tracing::info!("Kawakaze Backend - FreeBSD Jail Manager");
    tracing::info!("=======================================");

    // Check if running as root
    #[cfg(unix)]
    if unsafe { libc::getuid() } != 0 {
        tracing::warn!("Not running as root. Jail operations require root privileges.");
    }

    // Load configuration from default locations, or use defaults
    let config = match KawakazeConfig::load_defaults() {
        Ok(cfg) => {
            tracing::info!("Loaded configuration");
            tracing::info!("ZFS pool: {}", cfg.zfs_pool);
            tracing::info!("Database: {}", cfg.storage.database_path);
            cfg
        }
        Err(e) => {
            tracing::warn!("Failed to load configuration ({}), using defaults", e);
            KawakazeConfig::default()
        }
    };

    // Create jail manager with configuration (includes ZFS initialization)
    let manager = match JailManager::with_config(config) {
        Ok(m) => {
            tracing::info!("JailManager initialized with ZFS support");
            Arc::new(Mutex::new(m))
        }
        Err(e) => {
            tracing::error!("Failed to initialize JailManager: {}", e);
            tracing::error!("Please ensure:");
            tracing::error!("  1. A ZFS pool exists (run 'zpool list' to check)");
            tracing::error!("  2. The zfs_pool in config points to a valid pool");
            return Err(e.into());
        }
    };

    // Start the manager
    manager.lock().await.start().await?;

    // Create and run the socket server
    let socket_path = Arc::new("/var/run/kawakaze.sock".to_string());
    let server = kawakaze_backend::server::SocketServer::new(socket_path, manager);

    tracing::info!("Starting Kawakaze API server...");
    server.run().await?;

    Ok(())
}
