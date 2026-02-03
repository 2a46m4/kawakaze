# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

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
└── cli/                # CLI application crate
    ├── Cargo.toml
    ├── src/
    └── Dockerfile.example
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

The workspace contains a single member `cli` which provides a command-line tool for parsing Dockerfiles using the `dockerfile-parser` library.
