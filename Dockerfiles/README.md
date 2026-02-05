# Test Dockerfiles for Kawakaze

This directory contains example Dockerfiles to test the Kawakaze image build mechanism.

## Dockerfiles

### 1. nginx.dockerfile
A simple Nginx web server image that:
- Installs nginx from FreeBSD packages
- Creates a basic HTML welcome page
- Configures nginx to run in the foreground
- Listens on port 80

**Build:**
```bash
kawakaze build --name nginx-test Dockerfiles/nginx.dockerfile
```

**Run:**
```bash
kawakaze run -p 8080:80 nginx-test --name my-nginx
```

### 2. postgresql.dockerfile
A PostgreSQL database server image that:
- Installs PostgreSQL 16 server
- Initializes the database
- Configures remote access (for testing)
- Listens on port 5432

**Build:**
```bash
kawakaze build --name postgresql-test Dockerfiles/postgresql.dockerfile
```

**Run:**
```bash
kawakaze run -p 5432:5432 postgresql-test --name my-postgres
```

### 3. redis.dockerfile
A Redis cache server image that:
- Installs Redis from FreeBSD packages
- Configures Redis to bind to all interfaces
- Persists data to /var/db/redis
- Listens on port 6379

**Build:**
```bash
kawakaze build --name redis-test Dockerfiles/redis.dockerfile
```

**Run:**
```bash
kawakaze run -p 6379:6379 redis-test --name my-redis
```

### 4. simple-test.dockerfile
A minimal test image that validates basic Dockerfile features:
- Tests WORKDIR instruction
- Tests ENV variables
- Tests RUN commands
- Tests USER switching
- Tests VOLUME declaration
- Tests CMD execution

**Build:**
```bash
kawakaze build --name simple-test Dockerfiles/simple-test.dockerfile
```

**Run:**
```bash
kawakaze run simple-test --name test-container
```

## Prerequisites

Before building these images, you need to build the base FreeBSD image:

```bash
kawakaze build --name freebsd-15.0-release Dockerfile.base
```

## Testing the Build Mechanism

These Dockerfiles test various aspects of the image builder:

1. **FROM instruction** - All images inherit from freebsd-15.0-release
2. **LABEL instruction** - Metadata is set on all images
3. **ENV instruction** - Environment variables are configured
4. **RUN instruction** - Package installation and configuration
5. **WORKDIR instruction** - Working directory is set
6. **EXPOSE instruction** - Ports are documented
7. **VOLUME instruction** - Data volumes are declared
8. **USER instruction** - User switching (tested in simple-test and postgresql)
9. **CMD instruction** - Default commands are set
10. **ENTRYPOINT instruction** - Entrypoint script (tested in postgresql)

## Expected Behavior

When you build these images, you should see:
- Each instruction being processed
- Packages being installed
- Files being created
- Layers being built with ZFS snapshots
- A final image ID/UUID being generated

When you run containers from these images:
- nginx: Web server accessible on port 80
- postgresql: Database server accessible on port 5432
- redis: Cache server accessible on port 6379
- simple-test: Prints "Hello from Kawakaze!" and exits
