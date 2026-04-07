# Maelstrom

<img src="./.github/logo-banner.svg">

**Enterprise-grade, horizontally-scalable [Matrix](https://matrix.org) homeserver built in Rust.**

Maelstrom is a from-scratch Matrix homeserver designed for **clustered, high-availability deployments** from day one. It targets full **Matrix 2.0+** compliance (v1.18+) with dramatically lower resource usage than existing implementations.

General discussion: [#maelstrom-server:matrix.org](https://matrix.to/#/#maelstrom-server:matrix.org)

## Project Status

**Active development** -- complete architectural rewrite in progress. Not yet usable for production.

See [PROJECT.md](PROJECT.md) for the full development plan and phase tracking.

## Architecture

| Component | Technology | Purpose |
|-----------|-----------|---------|
| Language | Rust (2024 edition) | Core implementation |
| Web Framework | Axum | HTTP server, routing, middleware |
| Database | SurrealDB v3 (TiKV backend) | Event graph, state, users, rooms |
| Blob Storage | RustFS | S3-compatible media storage |
| Deployment | Docker, Kubernetes, Helm | Container orchestration |

### Design Principles

- **Stateless application layer** -- multiple Axum instances behind a load balancer, sharing one logical homeserver identity. Scale by adding nodes.
- **Graph-first data model** -- Matrix's event DAG, room membership, and relations modeled as SurrealDB graph relations for fast traversal and state resolution.
- **Storage trait abstraction** -- all database access through traits, enabling testability and future backend flexibility.
- **Zero-copy and CoW patterns** -- performance-optimized serialization and string handling throughout.

### Workspace Layout

```
maelstrom/
├── crates/
│   ├── maelstrom-core/         # Core types, events, state resolution, errors
│   ├── maelstrom-storage/      # Storage traits + SurrealDB implementation
│   ├── maelstrom-media/        # Media handling via S3 (RustFS)
│   ├── maelstrom-api/          # Client-Server API (Axum handlers)
│   ├── maelstrom-federation/   # Server-Server API
│   └── maelstrom-admin/        # Admin API and dashboard backend
├── src/main.rs                 # Binary entry point
├── tests/                      # Integration tests
├── config/                     # Configuration files
├── docker-compose.yml          # Full clustered stack (TiKV + SurrealDB + RustFS)
└── docker-compose.dev.yml      # Lightweight local dev stack
```

## Quick Start

### Prerequisites

- Rust 1.85+ (2024 edition)
- Docker and Docker Compose

### Development (single-node)

```bash
git clone https://github.com/maelstrom-rs/maelstrom.git && cd maelstrom

# Start SurrealDB + RustFS for local development
docker compose -f docker-compose.dev.yml up -d

# Copy and edit configuration
cp config/example.toml config/local.toml

# Build and run
cargo run --release
```

### Clustered (TiKV backend)

```bash
# Start full stack: PD + 3x TiKV + SurrealDB + RustFS
docker compose up -d

# Run Maelstrom (can run multiple instances behind a load balancer)
cargo run --release
```

### Running Tests

```bash
# Unit and integration tests
cargo test

# With full stack (federation, media, clustering tests)
docker compose up -d
cargo test --features integration
```

## Administration

### Configuration

Maelstrom is configured via TOML files. See `config/example.toml` for all options.

Key configuration areas:
- **Server**: bind address, public hostname, max request size
- **Database**: SurrealDB connection (endpoint, namespace, credentials)
- **Media**: RustFS/S3 endpoint, bucket name, upload limits, retention policies
- **Federation**: signing key paths, allow/deny lists
- **Logging**: level, format (JSON for production, pretty for dev)

### Docker Compose Services

The `docker-compose.yml` provides a production-like clustered environment:

| Service | Port | Purpose |
|---------|------|---------|
| `surrealdb` | 8000 | Database (connects to TiKV cluster) |
| `pd` | 2379 | TiKV Placement Driver |
| `tikv-0..2` | 20160+ | TiKV storage nodes (3-node Raft cluster) |
| `rustfs` | 9000 | S3-compatible blob storage |

### Health Checks

- `GET /_health/live` -- liveness probe (server is running)
- `GET /_health/ready` -- readiness probe (SurrealDB + RustFS connected)

### Monitoring

Maelstrom exports Prometheus metrics at `GET /metrics`:
- Request latency and throughput by endpoint
- Active sync connections
- Federation queue depth and destination health
- Database query latency
- Media storage usage

### Admin API

The admin API (under `/_maelstrom/admin/v1/`) provides:
- **User management**: list, create, suspend, lock, deactivate users
- **Room management**: list, inspect, shutdown, purge rooms
- **Media management**: usage stats, quarantine, retention policy enforcement
- **Federation**: destination health, queue status, blocklists
- **Reports**: abuse report management and actions

Admin endpoints require an admin-level access token.

## Matrix Spec Compliance

Maelstrom targets **100% compliance** with the stable Matrix specification, validated through the [Complement](https://github.com/matrix-org/complement) black-box test suite.

Target spec version: **Matrix v1.18+** including:
- Sliding Sync (Matrix 2.0 core)
- Threads, reactions, polls
- Spaces and room hierarchy
- End-to-end encryption (key management, cross-signing)
- Federation with all major homeservers
- Trust & safety (account suspension, invite blocking, policy servers)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Maelstrom by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
