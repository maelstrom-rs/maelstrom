# Maelstrom

<img src="./.github/logo-banner.svg">

**The open-source, enterprise-scale [Matrix](https://matrix.org) homeserver.**

Maelstrom is the only open-source Matrix homeserver built for horizontal scaling. It is a from-scratch Rust implementation with a stateless application layer, distributed graph storage via SurrealDB + TiKV, and true multi-instance clustering — no worker model, no per-process GIL limits, no single-database bottleneck.

General discussion: [#maelstrom-server:matrix.org](https://matrix.to/#/#maelstrom-server:matrix.org)

## Why Maelstrom

Today's Matrix scaling options are limited:

- **Synapse** (85% market share) scales via Python worker processes, each limited to one CPU core. At ~40 events/second, 25% of CPU is overhead before real work begins. Element's own engineering team calls this a hard ceiling — and their solution, Synapse Pro, is proprietary and closed-source.
- **Conduit/Continuwuity** (Rust) are fast on a single machine but use embedded RocksDB with no clustering capability.
- **Dendrite** (Go) is in maintenance mode with no production-ready horizontal scaling.

There is no open-source Matrix homeserver that offers simple horizontal scaling for large deployments. Maelstrom fills that gap:

- **True horizontal scaling** -- stateless Axum instances behind a load balancer, with chitchat gossip for cross-node ephemeral state. Scale by adding nodes. No worker types to configure, no process-specific routing, no external message broker.
- **Distributed storage** -- SurrealDB on TiKV provides automatic sharding, replication, and ACID transactions across a cluster. No single-PostgreSQL bottleneck.
- **Rust performance** -- no GIL, no garbage collector. A single instance handles what Synapse needs multiple workers for.
- **Full Matrix 2.0+** -- sliding sync, threads, reactions, spaces, E2EE, federation with Synapse/Dendrite/Conduit.
- **Built-in admin** -- web dashboard and JSON API for user, room, media, and federation management. Prometheus metrics. Runtime-configurable media retention.
- **Graph-native** -- the Matrix event DAG, room membership, and event relations (threads, reactions, edits) are modeled as SurrealDB graph edges, not flat tables with string foreign keys.

## Project Status

**Alpha** -- under active development. Core functionality works. Not yet recommended for production deployments carrying real user data.

Complement test suite: **147+/370 CS API tests passing** (39.7%). Registration, login, profile, and E2EE subsystems are solid. Room operations and sync are being hardened.

## Quick Start

### Docker (recommended)

```bash
git clone https://github.com/maelstrom-rs/maelstrom.git && cd maelstrom

# Start backing services (SurrealDB + RustFS media storage)
docker compose -f docker-compose.dev.yml up -d

# Copy and edit configuration
cp config/example.toml config/local.toml
# Edit server_name, bind_address, and credentials as needed

# Build and run
cargo build --release
./target/release/maelstrom
```

Maelstrom listens on port **8008** by default. Point your Matrix client at `http://your-server:8008`.

### First Login

The first user to register becomes the server administrator automatically. Alternatively, set `admin_user` in your config to bootstrap an admin account on startup:

```toml
[server]
admin_user = "admin"
```

## Configuration

All configuration lives in a single TOML file. Set `MAELSTROM_CONFIG` to override the default path (`config/local.toml`).

See [`config/example.toml`](config/example.toml) for all options with comments.

```toml
[server]
bind_address = "0.0.0.0:8008"
server_name = "example.com"          # Your domain — appears in user IDs (@user:example.com)
public_base_url = "https://example.com"

[database]
endpoint = "ws://localhost:8000"     # SurrealDB connection
namespace = "maelstrom"
database = "maelstrom"
username = "root"
password = "change-me"

[media]                              # Optional — omit to disable media uploads
endpoint = "http://localhost:9000"   # RustFS / S3-compatible storage
bucket = "maelstrom-media"
access_key = "maelstrom"
secret_key = "change-me"
region = "us-east-1"
max_age_days = 90                    # 0 = keep forever
sweep_interval_secs = 3600

[cluster]                            # Optional — omit for single-node
listen_addr = "0.0.0.0:7280"        # UDP address for gossip protocol
seed_nodes = ["node2:7280"]          # Peers to bootstrap cluster from
cluster_id = "maelstrom"             # Nodes with different IDs ignore each other
```

## Deployment Modes

### Single Node

For personal use, small teams, or evaluation. One Maelstrom process with SurrealDB using file-based storage. No `[cluster]` config needed.

| Service | Purpose | Default Port |
|---------|---------|-------------|
| Maelstrom | Homeserver | 8008 |
| SurrealDB | Database (SurrealKV file storage) | 8000 |
| RustFS | Media storage (optional) | 9000 |

```bash
docker compose -f docker-compose.dev.yml up -d
```

### Cluster

For production and high availability. Multiple Maelstrom instances behind a load balancer. SurrealDB backed by TiKV for distributed storage. Ephemeral state (typing indicators, presence) is synchronized across nodes via the [chitchat](https://github.com/quickwit-oss/chitchat) gossip protocol over UDP — no external message broker required.

| Service | Purpose | Default Port |
|---------|---------|-------------|
| Maelstrom (N instances) | Homeserver | 8008, 7280 (gossip) |
| SurrealDB | Database (TiKV backend) | 8000 |
| TiKV (3+ nodes) | Distributed storage | 20160 |
| PD | TiKV Placement Driver | 2379 |
| RustFS | Media storage | 9000 |

```bash
# Start the full stack (SurrealDB + TiKV + RustFS)
docker compose up -d
```

Add a `[cluster]` section to each node's config:

```toml
# Node 1 (maelstrom-1)
[cluster]
listen_addr = "0.0.0.0:7280"
seed_nodes = ["maelstrom-2:7280"]

# Node 2 (maelstrom-2)
[cluster]
listen_addr = "0.0.0.0:7280"
seed_nodes = ["maelstrom-1:7280"]

# Node 3+ only needs one seed — learns the rest via gossip
[cluster]
listen_addr = "0.0.0.0:7280"
seed_nodes = ["maelstrom-1:7280"]
```

In Kubernetes, use a headless service as the seed — chitchat resolves hostnames, so a single DNS entry returning all pod IPs works for automatic discovery.

Scale horizontally by running additional instances pointed at the same SurrealDB. All instances share one homeserver identity.

## Administration

### Admin Dashboard

Access the web dashboard at `/_maelstrom/admin/` (requires an admin account).

### Admin API

All admin operations are available as JSON endpoints under `/_maelstrom/admin/v1/`. Authenticate with a Bearer token from an admin user account.

| Endpoint | Description |
|----------|-------------|
| `GET /server/info` | Version, uptime, memory, database status |
| `GET /server/health` | Detailed service health |
| `GET /metrics` | Prometheus metrics (uptime, memory, DB status) |
| `GET /users/{userId}` | User details, devices, room count |
| `POST /users/{userId}/deactivate` | Deactivate account |
| `POST /users/{userId}/reactivate` | Reactivate account |
| `POST /users/{userId}/reset-password` | Reset password |
| `PUT /users/{userId}/admin` | Grant admin |
| `DELETE /users/{userId}/admin` | Revoke admin |
| `GET /rooms` | List rooms |
| `GET /rooms/{roomId}` | Room details and members |
| `POST /rooms/{roomId}/shutdown` | Remove all members |
| `GET /media/user/{userId}` | User's media files |
| `POST /media/{server}/{mediaId}/quarantine` | Quarantine media |
| `GET /media/retention` | View retention policy |
| `PUT /media/retention` | Update retention policy at runtime |
| `POST /media/retention/sweep` | Trigger immediate retention sweep |
| `GET /federation/stats` | Federation status |
| `GET /reports` | Abuse reports |

### Health Checks

For load balancer and Kubernetes probe configuration:

| Endpoint | Purpose | Checks |
|----------|---------|--------|
| `GET /_health/live` | Liveness probe | Process is running |
| `GET /_health/ready` | Readiness probe | Database connected |

### Monitoring

Prometheus metrics are available at `GET /_maelstrom/admin/v1/metrics` (admin auth required):

```
maelstrom_uptime_seconds
maelstrom_memory_used_bytes
maelstrom_memory_total_bytes
maelstrom_database_up
```

### Media Retention

Media retention can be configured in the TOML file or changed at runtime via the admin API:

```bash
# View current policy
curl -H "Authorization: Bearer $TOKEN" http://localhost:8008/_maelstrom/admin/v1/media/retention

# Set 90-day retention
curl -X PUT -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 90}' \
  http://localhost:8008/_maelstrom/admin/v1/media/retention

# Run an immediate cleanup
curl -X POST -H "Authorization: Bearer $TOKEN" \
  http://localhost:8008/_maelstrom/admin/v1/media/retention/sweep
```

### Federation

Federation is enabled automatically. Maelstrom generates an Ed25519 signing key on first startup and persists it in the database. Other servers can fetch your keys at `/_matrix/key/v2/server`.

For federation to work, your server must be reachable from the internet. Configure DNS:

1. Set an SRV record: `_matrix-fed._tcp.example.com → your-server:8448`
2. Or serve `.well-known/matrix/server` returning `{"m.server": "matrix.example.com:8448"}`
3. Or run on port 8448 with your `server_name` resolving to your IP

## Logging

Maelstrom uses structured logging. Set the log level via the `RUST_LOG` environment variable:

```bash
RUST_LOG=info ./maelstrom                    # Normal operation
RUST_LOG=debug ./maelstrom                   # Verbose
RUST_LOG=maelstrom=debug,surrealdb=warn ./maelstrom  # Debug Maelstrom, quiet SurrealDB
```

## Architecture

| Layer | Technology |
|-------|-----------|
| Runtime | Rust + Tokio + Axum |
| Database | SurrealDB v3 (graph model) |
| Media | RustFS (S3-compatible) |
| Ephemeral state | In-memory (DashMap) + chitchat gossip for clustering |
| Admin dashboard | Askama SSR + Datastar |
| Testing | Complement (Matrix spec conformance) |

All data is stored in SurrealDB using its graph model:
- **Events** stored with DAG edges (`event --prev_event--> event`, `event --auth_event--> event`)
- **Membership** as graph relations (`user --member_of--> room`)
- **Reactions, threads, edits** as graph relations (`event --relates_to--> event`)

Ephemeral data (typing indicators, presence) lives in-memory using lock-free `DashMap` structures. In cluster mode, the [chitchat](https://github.com/quickwit-oss/chitchat) Scuttlebutt gossip protocol propagates ephemeral state across nodes over UDP with automatic TTL-based expiry — no external message broker needed.

## Building from Source

Requires Rust 1.85+ (2024 edition).

```bash
cargo build --release
```

The binary is at `target/release/maelstrom`. It is fully self-contained — no runtime dependencies beyond libc.

## Running Tests

```bash
# Unit and integration tests
cargo test

# Matrix spec conformance (requires Docker + Go)
make complement
```

## License

Licensed under [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
