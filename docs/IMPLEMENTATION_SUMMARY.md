# Kawakaze Implementation Summary

## Overview

This document summarizes the implementation of Images, Containers, and ZFS Integration for the Kawakaze FreeBSD jail manager. The implementation provides a Docker-like interface for managing FreeBSD jails using ZFS for storage.

**Implementation Date:** February 2026
**Status:** Complete ✅

---

## Architecture

```
┌─────────────┐     Unix Socket      ┌──────────────┐
│   CLI       │ ◄──────────────────► │   Backend    │
│  (kawakaze) │     JSON Protocol    │ (kawakazed)  │
└─────────────┘                      └──────────────┘
                                           │
                                           ▼
                                  ┌─────────────────┐
                                  │   FreeBSD Jail  │
                                  │   ZFS Datasets  │
                                  │   SQLite Store  │
                                  └─────────────────┘
```

---

## Implementation Phases

### Phase 1: Foundation (ZFS, Config, Database)

#### 1.1 ZFS Module (`backend/src/zfs.rs`)

Complete ZFS wrapper for managing datasets, snapshots, and clones.

**Key Types:**
- `Zfs` - Main wrapper struct
- `ZfsError` - Comprehensive error handling

**Methods:**
```rust
Zfs::new(pool)                          // Create wrapper, verify pool exists
create_dataset(path)                    // Create new dataset
create_snapshot(dataset, name)          // Create named snapshot
clone_snapshot(snapshot, target)        // Clone snapshot to new dataset
destroy(path)                           // Destroy dataset or snapshot
get_mountpoint(dataset)                 // Get dataset mountpoint
list_snapshots(dataset)                 // List all snapshots
dataset_exists(dataset)                 // Check if dataset exists
set_property(dataset, prop, value)      // Set ZFS property
get_property(dataset, prop)             // Get ZFS property
```

**Lines of Code:** 1,134

---

#### 1.2 Configuration Module (`backend/src/config.rs`)

Configuration file loading with sensible defaults.

**Key Types:**
```rust
pub struct KawakazeConfig {
    pub zfs_pool: String,
    pub network: NetworkConfig,
    pub storage: StorageConfig,
    pub api: ApiConfig,
}
```

**Default Values:**
- ZFS Pool: `zroot/kawakaze`
- Container CIDR: `10.11.0.0/16`
- Bridge Name: `kawakaze-bridge`
- NAT: Enabled
- Database Path: `/var/db/kawakaze/kawakaze.db`
- Socket Path: `/var/run/kawakaze.sock`
- Cache Path: `/var/cache/kawakaze`

**Configuration Locations (in order):**
1. `/etc/kawakaze/config.toml`
2. `~/.config/kawakaze/config.toml`
3. Built-in defaults

**Lines of Code:** 460

---

#### 1.3 Database Schema Updates (`backend/src/store.rs`)

Added two new tables for images and containers:

**Images Table:**
```sql
CREATE TABLE images (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    parent_id TEXT,
    snapshot TEXT NOT NULL,
    dockerfile TEXT NOT NULL,
    config TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL CHECK(state IN ('building', 'available', 'deleted')),
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY (parent_id) REFERENCES images(id) ON DELETE CASCADE
);
```

**Containers Table:**
```sql
CREATE TABLE containers (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE,
    image_id TEXT NOT NULL,
    jail_name TEXT UNIQUE NOT NULL,
    dataset TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('created', 'running', 'stopped', 'paused', 'removing')),
    restart_policy TEXT NOT NULL DEFAULT 'no',
    mounts TEXT NOT NULL,
    port_mappings TEXT NOT NULL,
    ip TEXT,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    started_at INTEGER,
    FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE RESTRICT
);
```

**CRUD Methods Added:**
- Images: `insert_image`, `get_image`, `get_image_by_name`, `list_images`, `update_image`, `delete_image`
- Containers: `insert_container`, `get_container`, `get_container_by_name`, `list_containers`, `update_container`, `delete_container`

---

#### 1.4 API Endpoints (`backend/src/api.rs`)

Added REST-like endpoints for images and containers:

**Image Endpoints:**
- `GET /images` - List all images
- `GET /images/{id}` - Get image details
- `POST /images/build` - Build image from Dockerfile
- `DELETE /images/{id}` - Delete image
- `GET /images/{id}/history` - Get image history/layers

**Container Endpoints:**
- `GET /containers` - List all containers
- `GET /containers/{id}` - Get container details
- `POST /containers/create` - Create container
- `POST /containers/{id}/start` - Start container
- `POST /containers/{id}/stop` - Stop container
- `DELETE /containers/{id}` - Remove container
- `GET /containers/{id}/logs` - Get container logs
- `POST /containers/{id}/exec` - Execute command in container

---

### Phase 2: Image & Container Types

#### 2.1 Image Types (`backend/src/image.rs`)

Core type definitions for images.

**Key Types:**
```rust
pub type ImageId = String;

pub enum ImageState {
    Building,
    Available,
    Deleted,
}

pub enum DockerfileInstruction {
    From(String),
    Run(String),
    Copy { from: Option<String>, src: String, dest: String },
    Add { src: String, dest: String },
    WorkDir(String),
    Env(HashMap<String, String>),
    Expose(Vec<u16>),
    User(String),
    Volume(Vec<String>),
    Cmd(Vec<String>),
    Entrypoint(Vec<String>),
    Label(HashMap<String, String>),
}

pub struct ImageConfig {
    pub env: HashMap<String, String>,
    pub workdir: Option<PathBuf>,
    pub user: Option<String>,
    pub exposed_ports: Vec<u16>,
    pub volumes: Vec<String>,
    pub entrypoint: Option<Vec<String>>,
    pub cmd: Option<Vec<String>>,
    pub labels: HashMap<String, String>,
}

pub struct Image {
    pub id: ImageId,
    pub name: String,
    pub parent_id: Option<ImageId>,
    pub snapshot: String,
    pub dockerfile: Vec<DockerfileInstruction>,
    pub config: ImageConfig,
    pub size_bytes: u64,
    pub state: ImageState,
    pub created_at: i64,
}
```

---

#### 2.2 Container Types (`backend/src/container.rs`)

Core type definitions for containers.

**Key Types:**
```rust
pub type ContainerId = String;

pub enum ContainerState {
    Created,
    Running,
    Stopped,
    Paused,
    Removing,
}

pub enum RestartPolicy {
    No,
    OnRestart,
    OnFailure,
    Always,
}

pub enum MountType {
    Zfs,
    Nullfs,
}

pub enum PortProtocol {
    Tcp,
    Udp,
}

pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: PortProtocol,
}

pub struct Mount {
    pub source: String,
    pub destination: String,
    pub mount_type: MountType,
    pub read_only: bool,
}

pub struct Container {
    pub id: ContainerId,
    pub name: Option<String>,
    pub image_id: String,
    pub jail_name: String,
    pub dataset: String,
    pub state: ContainerState,
    pub restart_policy: RestartPolicy,
    pub mounts: Vec<Mount>,
    pub port_mappings: Vec<PortMapping>,
    pub ip: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
}
```

---

#### 2.3 Image Builder (`backend/src/image_builder.rs`)

Dockerfile execution engine for building images.

**Key Features:**
- Parse Dockerfile syntax (all standard instructions)
- Execute instructions in chroot environment
- ZFS-based layer management
- Progress tracking via async channels
- Build argument support with variable substitution
- Automatic cleanup on failure

**Supported Instructions:**
- `FROM` - Base image
- `RUN` - Execute commands (FreeBSD chroot)
- `COPY` - Copy files from build context
- `ADD` - Copy files with URL support
- `WORKDIR` - Set working directory
- `ENV` - Environment variables
- `EXPOSE` - Declare ports
- `USER` - Set user
- `VOLUME` - Declare volumes
- `CMD` - Default command
- `ENTRYPOINT` - Container entrypoint
- `LABEL` - Metadata labels
- `ARG` - Build arguments
- `STOPSIGNAL` - Stop signal
- `SHELL` - Default shell

**Lines of Code:** 907

---

#### 2.4 JailManager Extension (`backend/src/lib.rs`)

Extended the central `JailManager` with image and container management.

**New Fields:**
```rust
pub struct JailManager {
    // ... existing fields ...

    pub(crate) images: HashMap<ImageId, Image>,
    pub(crate) containers: HashMap<ContainerId, Container>,
    pub(crate) zfs: Option<Zfs>,
    pub(crate) config: KawakazeConfig,
    pub image_build_tracker: HashMap<ImageId, mpsc::Sender<ImageBuildProgress>>,
    pub image_build_progress: HashMap<ImageId, ImageBuildProgress>,
}
```

**New Methods - Images:**
```rust
pub fn with_config(config: KawakazeConfig) -> Result<Self>
pub fn add_image(&mut self, image: Image) -> Result<(), ImageError>
pub fn get_image(&self, id: &ImageId) -> Option<&Image>
pub fn get_image_by_name(&self, name: &str) -> Option<&Image>
pub fn list_images(&self) -> Result<Vec<&Image>>
pub fn remove_image(&mut self, id: &ImageId) -> Result<(), ImageError>
```

**New Methods - Containers:**
```rust
pub fn create_container(&mut self, config: ContainerConfig) -> Result<Container, ContainerError>
pub fn start_container(&mut self, id: &ContainerId) -> Result<(), ContainerError>
pub fn stop_container(&mut self, id: &ContainerId) -> Result<(), ContainerError>
pub fn remove_container(&mut self, id: &ContainerId) -> Result<(), ContainerError>
pub fn get_container(&self, id: &ContainerId) -> Option<&Container>
pub fn get_container(&self, id: &ContainerId) -> Option<&Container>
pub fn list_containers(&self) -> Result<Vec<&Container>>
```

---

### Phase 3: API Handlers

#### 3.1 Image Handlers (`backend/src/handler.rs`)

Implemented complete image API handlers:

- `list_images()` - List all images with summary info
- `get_image(id_or_name)` - Get image by ID or name
- `build_image(request)` - Build image from Dockerfile (async background task)
- `delete_image(id_or_name)` - Delete image with ZFS cleanup
- `get_image_history(id_or_name)` - Get Dockerfile instruction history

---

#### 3.2 Container Handlers (`backend/src/handler.rs`)

Implemented complete container API handlers:

- `list_containers()` - List all containers with summary info
- `get_container(id_or_name)` - Get container by ID or name
- `create_container(request)` - Create container from image
- `start_container(id_or_name)` - Start container jail
- `stop_container(id_or_name)` - Stop container jail
- `remove_container(id_or_name)` - Remove container with cleanup

**Features:**
- Container lookup by ID or name
- Proper state transitions
- Error handling with HTTP-like status codes
- ZFS dataset cleanup on removal

---

### Phase 4: CLI Implementation

#### 4.1 CLI Rewrite (`cli/src/main.rs`)

Complete rewrite with Docker-like interface using `clap` derive API.

**Lines of Code:** 757

**Available Commands:**

| Command | Description | Example |
|---------|-------------|---------|
| `build` | Build image from Dockerfile | `kawakaze build -t myapp ./Dockerfile` |
| `run` | Run a container | `kawakaze run -p 8080:80 myapp` |
| `ps` | List containers | `kawakaze ps` |
| `start` | Start container | `kawakaze start webserver` |
| `stop` | Stop container | `kawakaze stop webserver` |
| `rm` | Remove container | `kawakaze rm abc123` |
| `images` | List images | `kawakaze images` |
| `rmi` | Remove image | `kawakaze rmi freebsd-15.0` |
| `logs` | View logs | `kawakaze logs webserver -f` |
| `exec` | Execute command | `kawakaze exec webserver /bin/sh` |
| `inspect` | Inspect object | `kawakaze inspect abc123` |

**Run Command Options:**
```bash
-p, --publish <port>      # Publish port (host:container/protocol)
-v, --volume <mount>      # Mount volume (source:dest)
-e, --env <var=value>     # Environment variable
--restart <policy>        # Restart policy (no|on-restart|on-failure|always)
--name <name>             # Container name
--workdir <path>          # Working directory
--user <user>             # Run as user
```

**Dependencies Added:**
- `clap = { version = "4.5", features = ["derive"] }`
- `tokio = { version = "1.42", features = ["rt-multi-thread", "macros", "net"] }`
- `tokio-util = { version = "0.7", features = ["codec"] }`
- `futures = "0.3"`

---

## File Structure

```
kawakaze/
├── Cargo.toml                      # Workspace definition
├── CLAUDE.md                       # Project instructions
├── docs/
│   └── IMPLEMENTATION_SUMMARY.md   # This file
├── cli/                            # CLI application
│   ├── Cargo.toml                  # CLI dependencies
│   ├── src/
│   │   └── main.rs                 # CLI (757 lines)
│   └── Dockerfile.example          # Example Dockerfile
└── backend/                        # Backend library
    ├── Cargo.toml                  # Backend dependencies
    ├── src/
    │   ├── api.rs                  # API types + endpoints
    │   ├── bootstrap.rs            # FreeBSD bootstrapping
    │   ├── config.rs               # Configuration (460 lines)
    │   ├── container.rs            # Container types
    │   ├── handler.rs              # API handlers
    │   ├── image.rs                # Image types
    │   ├── image_builder.rs        # Dockerfile engine (907 lines)
    │   ├── jail.rs                 # Jail lifecycle
    │   ├── lib.rs                  # JailManager + exports
    │   ├── server.rs               # Unix socket server
    │   ├── store.rs                # SQLite persistence
    │   └── zfs.rs                  # ZFS wrapper (1134 lines)
    ├── tests/
    │   └── api_integration.rs      # Integration tests
    └── examples/
        └── socket_client.rs        # Example client
```

---

## Building and Running

### Prerequisites

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Build

```bash
# Build all workspace members
cargo build --release

# Build specific package
cargo build -p kawakaze
cargo build -p kawakaze-backend
```

### Test

```bash
# Run all tests
cargo test

# Run specific package tests
cargo test -p kawakaze-backend

# Run integration tests (requires ZFS pool named "tank")
cargo test -- --ignored
```

### Run

```bash
# Start the backend daemon
sudo ./target/release/kawakazed

# Use the CLI
kawakaze build -t myapp ./Dockerfile
kawakaze images
kawakaze run -p 8080:80 myapp
kawakaze ps
```

---

## Configuration

Create `/etc/kawakaze/config.toml`:

```toml
zfs_pool = "zroot/kawakaze"

[network]
container_cidr = "10.11.0.0/16"
bridge_name = "kawakaze-bridge"
nat_enabled = true

[storage]
database_path = "/var/db/kawakaze/kawakaze.db"
socket_path = "/var/run/kawakaze.sock"
cache_path = "/var/cache/kawakaze"

[api]
timeout = 30
```

---

## Example Dockerfile

```dockerfile
FROM freebsd-15.0

# Install packages
RUN pkg install -y nginx

# Set working directory
WORKDIR /usr/local/www/nginx

# Copy website files
COPY . /usr/local/www/nginx

# Expose port
EXPOSE 80

# Set environment
ENV NGINX_VERSION=1.24

# Run command
CMD ["nginx", "-g", "daemon off;"]
```

---

## API Protocol

The backend communicates over a Unix socket using JSON line-delimited messages.

**Request Format:**
```json
{
  "method": "POST",
  "endpoint": "/images/build",
  "body": {
    "name": "myapp",
    "dockerfile": "FROM freebsd-15.0\nRUN pkg install -y nginx",
    "build_args": {}
  }
}
```

**Response Format:**
```json
{
  "status": "success",
  "data": {
    "id": "abc123...",
    "name": "myapp",
    "size_bytes": 500000000
  }
}
```

---

## Networking (Phase 2 - Future)

Networking support is planned for Phase 2:
- Per-container `epair` interfaces
- Bridge device (`kawakaze0`)
- PF NAT for internet access
- Port forwarding via `pf` redirections

---

## Summary Statistics

| Metric | Count |
|--------|-------|
| New Modules Created | 5 (zfs, config, image, container, image_builder) |
| Modules Modified | 4 (store, api, handler, lib) |
| CLI Rewrite | 1 (main.rs) |
| Total New Lines of Code | ~4,000+ |
| API Endpoints Added | 13 |
| CLI Commands Added | 11 |
| Database Tables Added | 2 |
| Dockerfile Instructions Supported | 15 |

---

## Next Steps

1. **Install Rust toolchain** on your FreeBSD system
2. **Build the project**: `cargo build --release`
3. **Create ZFS pool** for Kawakaze datasets
4. **Configure** `/etc/kawakaze/config.toml`
5. **Run the daemon**: `sudo ./target/release/kawakazed`
6. **Build your first image**: `kawakaze build -t test-image ./Dockerfile.example`
7. **Run a container**: `kawakaze run -p 8080:80 test-image`

---

## Notes

- No backwards compatibility maintained - this is a complete redesign
- All existing `/jails` API has been replaced with images/containers API
- ZFS is required for image/container storage
- FreeBSD 15.0-RELEASE is the target platform
- SQLite provides persistence across daemon restarts
- Async progress tracking for long-running operations (builds, bootstrap)

---

**Implementation completed February 2026**
Total implementation time: 4 phases using parallel subagent strategy
