# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Use subagents for distinct phases.

## Project Overview

Kawakaze is a jail manager for FreeBSD. The user interacts with Kawakaze using a CLI tool. 

### CLI
The CLI can create, destroy, and manage jails. It is able to parse Dockerfiles and create containers that way. 

### Backend
The backend is the section that actually manages the jails. It communicates with clients through a unix socket. It should interface with the libjail library.

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

The backend can bootstrap jails with a complete FreeBSD base system:

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
4. Generate minimal config files (`rc.conf`, `resolv.conf`, `hosts`)
5. Cache tarball at `/var/cache/kawakaze/` for future use

Bootstrap runs asynchronously in background - the API returns immediately after starting the operation.
