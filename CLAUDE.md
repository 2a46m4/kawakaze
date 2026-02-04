# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Use subagents for distinct phases. It is important to write unit and integration tests for all the code that you write.

## Project Overview

Kawakaze is a jail manager for FreeBSD. The user interacts with Kawakaze using a CLI tool. 

### Concepts

There is a concept of Images and Containers.

#### Images
Images are blueprints of containers that can be produced by committing changes to a container, or by being created from a Dockerfile, which specifies the steps required to produce the image. Images can be created from other images, by using the FROM keyword in the Dockerfile. 

For now, a local image repository is all that is needed.

Implementation wise, the best way to implement this is to use ZFS snapshots.

#### Containers
Containers are instantiations of images. These exist as running jails. Their lifetime should be managed by the backend, with distinct restart policies (on-restart, on-fail, noop)

Implementation wise they can be promoted clones of a ZFS snapshot (thick jails) or regular clones. For ease of implementation, only thick jails should be considered for now.

The user should also be able to mount ZFS datasets or regular filepaths via nullfs. 

The containers should have an IP space of 10.11.0.0/16. Every container should be allocated an epair to connect to the internet. The pf should perform NAT on a bridge device. 

The container should be allowed to expose ports that can be redirected.

### CLI
The CLI can create, destroy, and manage jails. 

The user should be able to:
- Build images from Dockerfiles
- Instantiate containers
- Start and stop containers

The UI should be similar to Podman or Docker. A unique UUID should be generated for every container, along with a name and the image that it is running.

### Backend
The backend is the section that actually manages the jails. It communicates with clients through a unix socket. It should interface with the libjail library. The majority of the work should be done here, with the CLI being a relatively thin wrapper over the API.

## Workspace Structure

```
kawakaze/
├── Cargo.toml          # Workspace definition
├── CLAUDE.md           # This file
├── cli/                # CLI application crate
│   ├── Cargo.toml
│   ├── src/
│   └── Dockerfile.example
└── backend/            # Backend library crate
    ├── Cargo.toml
    ├── src/
    │   ├── api.rs          # API types (Request, Response, Endpoint)
    │   ├── bootstrap.rs    # FreeBSD base system bootstrapping
    │   ├── handler.rs      # API request handlers
    │   ├── jail.rs         # Jail lifecycle management
    │   ├── lib.rs          # Library entry point & JailManager
    │   ├── server.rs       # Unix socket server
    │   ├── store.rs        # SQLite persistence layer
    │   └── bin/
    │       └── kawakazed.rs # Backend daemon binary
    ├── tests/
    │   └── api_integration.rs
    └── examples/
        └── socket_client.rs
```

## Common Commands

### Build all workspace members
```bash
cargo build
```

### Build specific package
```bash
cargo build -p kawakaze
```

### Run the CLI
```bash
cargo run -p kawakaze -- <dockerfile-path>
# Example: cargo run -p kawakaze -- cli/Dockerfile.example
```

### Run tests
```bash
cargo test
```

### Check code without building
```bash
cargo check
```

### Lint
```bash
cargo clippy
```

### Format code
```bash
cargo fmt
```

## Architecture

The workspace contains two members:

### `backend` crate
Core library that manages FreeBSD jails. Provides:
- Jail lifecycle management (create, start, stop, destroy)
- JSON-over-Unix-socket API for client communication
- SQLite persistence for jail configurations
- FreeBSD base system bootstrapping (download + extract base.txz)
- Progress tracking for async bootstrap operations

Key modules:
- `jail.rs` - Direct FreeBSD jail operations using libc
- `api.rs` - REST-like API types (Request/Response/Endpoint)
- `handler.rs` - Request handlers that implement the API
- `server.rs` - Unix socket server with JsonCodec
- `store.rs` - SQLite persistence layer
- `bootstrap.rs` - FreeBSD base system bootstrapping
- `image_builder.rs` - Dockerfile-to-image builder with ZFS layer management
- `image.rs` - Image data structures and Dockerfile instruction types
- `container.rs` - Container lifecycle and management

### `cli` crate
Command-line interface that communicates with the backend daemon. Can:
- Parse Dockerfiles and create jails from them
- Communicate with backend via Unix socket at `/var/run/kawakaze.sock`
- Provide user-friendly commands for jail management

### Communication Pattern
```
CLI → Unix Socket → Backend Daemon → FreeBSD jail(2) syscalls
```

The backend runs as a daemon (`kawakazed`) that listens on a Unix socket. The CLI connects to this socket to send JSON requests and receive responses.

## FreeBSD Jail Bootstrapping

The backend can bootstrap jails with a complete FreeBSD base system. Bootstrapping can be done either via API endpoints or via the `BOOTSTRAP` Dockerfile instruction when building images.

### Dockerfile BOOTSTRAP Instruction

The `BOOTSTRAP` instruction can be used in Dockerfiles to automatically bootstrap a FreeBSD base system during image build. This is particularly useful for creating base images from scratch.

**Syntax:**
```dockerfile
FROM scratch
BOOTSTRAP [VERSION] [ARCHITECTURE] [MIRROR]
```

**Parameters:**
- `VERSION` - Optional: FreeBSD version (e.g., "15.0-RELEASE"). Auto-detected from host if not specified.
- `ARCHITECTURE` - Optional: Architecture (e.g., "amd64", "aarch64"). Auto-detected from host if not specified.
- `MIRROR` - Optional: Custom mirror URL. Uses official FreeBSD mirrors if not specified.

**Examples:**

Basic base image with auto-detection:
```dockerfile
FROM scratch
BOOTSTRAP
WORKDIR /root
```

Specify a specific FreeBSD version:
```dockerfile
FROM scratch
BOOTSTRAP 15.0-RELEASE
```

Specify version and architecture:
```dockerfile
FROM scratch
BOOTSTRAP 14.2-RELEASE amd64
```

Use a custom mirror:
```dockerfile
FROM scratch
BOOTSTRAP 15.0-RELEASE amd64 https://mirror.example.com/freebsd
```

**Example base image (`Dockerfile.base`):**
```dockerfile
# Base FreeBSD 15.0-RELEASE image
FROM scratch

# Bootstrap FreeBSD base system
# Downloads and installs the complete FreeBSD base system
BOOTSTRAP

# Set working directory
WORKDIR /root
```

**Note:** When building an image with `BOOTSTRAP`, the base system is downloaded during the build process and cached in `/var/cache/kawakaze/` for future builds. This significantly speeds up subsequent builds.

### API Endpoints

**Create jail with bootstrap:**
```json
POST /jails
{
  "name": "webserver",
  "path": "/jails/webserver",
  "ip": "192.168.1.100",
  "bootstrap": {
    "version": "15.0-RELEASE",    // Optional: auto-detected from host
    "architecture": "amd64",       // Optional: auto-detected from host
    "mirror": null,                // Optional: custom mirror URL
    "no_cache": false              // Optional: force re-download
  }
}
```

**Bootstrap existing jail:**
```json
POST /jails/{name}/bootstrap
{
  "version": "15.0-RELEASE",
  "architecture": "amd64",
  "mirror": null,
  "no_cache": false
}
```

**Check bootstrap progress:**
```json
GET /jails/{name}/bootstrap/status

Response:
{
  "status": "extracting",          // downloading|verifying|extracting|configuring|complete|error
  "progress": 65,                  // 0-100 percentage
  "current_step": "Extracting FreeBSD base system...",
  "version": "15.0-RELEASE",
  "architecture": "amd64"
}
```

### Bootstrap Process

1. Download official FreeBSD `base.txz` from CDN (~150MB compressed, ~500MB extracted)
2. Verify SHA256 checksum
3. Extract to jail path using tar + xz
4. Generate configuration files:
   - `/etc/rc.conf` - Basic RC configuration
   - `/etc/resolv.conf` - DNS configuration
   - `/etc/hosts` - Hostname mapping
   - `/etc/profile` - System-wide shell profile with PATH
   - `/root/.profile` - Root user profile with PATH
   - `/root/.cshrc` - Root user csh/tcsh configuration with PATH
5. Cache tarball at `/var/cache/kawakaze/` for future use

Bootstrap runs asynchronously in background - the API returns immediately after starting the operation.

### PATH Configuration

When bootstrapping a jail, Kawakaze automatically configures the PATH environment variable in shell profiles:

- **System-wide profile** (`/etc/profile`): Sets `PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin:~/bin`
- **Root user profile** (`/root/.profile`): Sets the same PATH for sh/bash shells
- **Root user cshrc** (`/root/.cshrc`): Sets the same PATH for csh/tcsh shells

This ensures that commands like `ls`, `uname`, and other FreeBSD utilities work correctly when executing commands in containers. The PATH is also passed automatically when using the `exec` command, so even if shell profiles aren't loaded, commands will work properly.

**Example:**
```bash
# These commands now work correctly in bootstrapped containers
kawakaze exec <container-id> uname -a
kawakaze exec <container-id> ls -la /root
```

## Dockerfile Instructions

Kawakaze supports a subset of Dockerfile instructions for building images:

### Supported Instructions

- `FROM <image>` - Specify base image (use `scratch` for empty base)
- `BOOTSTRAP [VERSION] [ARCH] [MIRROR]` - Bootstrap FreeBSD base system
- `RUN <command>` - Execute command during build
- `COPY <src> <dest>` - Copy files from build context
- `ADD <src> <dest>` - Copy files (with URL support)
- `WORKDIR <path>` - Set working directory
- `ENV <key> <value>` - Set environment variables
- `EXPOSE <port> ...` - Expose ports
- `USER <username>` - Set user for RUN/CMD/ENTRYPOINT
- `VOLUME <path> ...` - Create mount points
- `CMD <command>` - Default command to run
- `ENTRYPOINT <command>` - Container entrypoint
- `LABEL <key> <value>` - Add metadata labels
- `ARG <name>[=default]` - Build-time variables
- `STOPSIGNAL <signal>` - Signal to stop container
- `SHELL <command>` - Default shell for RUN commands

### Special Instructions

**`BOOTSTRAP`** - Kawakaze-specific instruction to bootstrap a FreeBSD base system during image build. See "FreeBSD Jail Bootstrapping" section above for details.

### Example Dockerfiles

**Simple base image:**
```dockerfile
FROM scratch
BOOTSTRAP
WORKDIR /root
```

**Web server image:**
```dockerfile
FROM freebsd-base
RUN pkg install -y nginx
EXPOSE 80 443
CMD ["nginx", "-g", "daemon off;"]
```

**Development environment:**
```dockerfile
FROM freebsd-base
RUN pkg install -y rust git vim
WORKDIR /workspace
ENV PATH=/usr/local/bin:$PATH
VOLUME /workspace
```
- Always use descriptive names
- After finishing a feature, make sure to write unit tests, integration tests, and system tests for it. You should test the behaviour of every edge case you can think of to make sure that it's as expected. Use additional test case libraries, or coverage libraries if you need to.
- Update CLAUDE.md if the information is out of date or if there is anything important that is added
- Write unit, integration, and system tests for any code that you write. Make sure that all paths are covered.
- Commit the code whenever a feature has been added and it's been thoroughly tested.
- You don't need to preserve any backwards compatibility or worry about deleting existing containers or images.