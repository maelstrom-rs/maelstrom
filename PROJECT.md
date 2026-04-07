# Maelstrom Project Plan

> Enterprise-Grade Clustered Matrix Homeserver — Complete Rewrite
> Last updated: 2026-04-07

---

## Architecture Overview

### Technology Stack

| Component | Technology | Version | Purpose |
|-----------|-----------|---------|---------|
| Language | Rust | 2024 edition | Core implementation |
| Web Framework | Axum | 0.8.x | HTTP server, routing, middleware |
| Middleware | Tower / Tower-HTTP | 0.5.x / 0.6.x | Service layers, CORS, compression, tracing |
| Database | SurrealDB | 3.x | Event graph, state, users, rooms (TiKV backend for clustering) |
| Blob Storage | RustFS | 1.0.0-alpha | S3-compatible media storage (accessed via aws-sdk-s3) |
| S3 Client | aws-sdk-s3 | 1.x | Interface to RustFS |
| Serialization | serde / serde_json | 1.x | Zero-copy where possible |
| Matrix Types | ruma | latest | Matrix identifiers, event types, canonical JSON |
| Async Runtime | Tokio | 1.x | Async I/O |
| Logging | tracing / tracing-subscriber | 0.1.x / 0.3.x | Structured logging + OpenTelemetry |
| Metrics | metrics / metrics-exporter-prometheus | 0.x | Prometheus-compatible metrics |
| Testing | Rust test framework + Complement | — | Unit/integration in `tests/`, black-box via Complement |
| Deployment | Docker, Docker Compose, Helm | — | Container orchestration |

### Workspace Structure

```
maelstrom/
├── Cargo.toml                    # Workspace root
├── PROJECT.md                    # This file
├── maelstrom-product-spec.md     # Product specification
├── docker-compose.yml            # TiKV cluster + RustFS + SurrealDB for dev/test
├── docker-compose.dev.yml        # Lightweight single-node dev setup
├── Dockerfile                    # Maelstrom server image (also used by Complement)
├── config/
│   ├── default.toml              # Default configuration
│   └── example.toml              # Documented example config
├── crates/
│   ├── maelstrom-core/           # Core types, Matrix events, state resolution, errors
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs          # MatrixError, ErrorCode enum
│   │       ├── identifiers.rs    # Re-exports and extensions on ruma identifiers
│   │       ├── events/           # Matrix event types and canonical JSON
│   │       │   ├── mod.rs
│   │       │   ├── pdu.rs        # Persistent Data Unit (core event structure)
│   │       │   ├── room.rs       # Room event types
│   │       │   └── state.rs      # State event handling
│   │       ├── state/            # State resolution algorithms
│   │       │   ├── mod.rs
│   │       │   ├── v2.rs         # State resolution v2 (room versions 2+)
│   │       │   └── room_version.rs
│   │       └── signatures/       # Event signing and verification
│   │           ├── mod.rs
│   │           └── keys.rs
│   ├── maelstrom-storage/        # Storage abstraction + SurrealDB implementation
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs         # Storage trait definitions (UserStore, RoomStore, EventStore, etc.)
│   │       ├── surreal/          # SurrealDB implementation
│   │       │   ├── mod.rs
│   │       │   ├── connection.rs # Connection pool, namespace/db setup
│   │       │   ├── schema.rs     # SurrealQL schema definitions (DEFINE TABLE, RELATE, indexes)
│   │       │   ├── users.rs      # User CRUD operations
│   │       │   ├── rooms.rs      # Room CRUD, membership graph queries
│   │       │   ├── events.rs     # Event storage, DAG traversal, timeline queries
│   │       │   ├── state.rs      # Room state snapshots and resolution cache
│   │       │   ├── sync.rs       # Sync token tracking, since-token queries
│   │       │   ├── devices.rs    # Device management
│   │       │   ├── keys.rs       # E2EE key storage
│   │       │   └── media.rs      # Media metadata (blob refs to RustFS)
│   │       └── mock.rs           # Mock storage for unit tests
│   ├── maelstrom-media/          # Media handling via S3 (RustFS)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── client.rs         # S3 client wrapper (aws-sdk-s3)
│   │       ├── upload.rs         # Upload handling, content-type validation
│   │       ├── download.rs       # Download, range requests
│   │       ├── thumbnail.rs      # Thumbnail generation
│   │       └── retention.rs      # Retention policy engine (used by admin tools)
│   ├── maelstrom-api/            # Axum server: CS API routes and handlers
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── router.rs         # Route tree construction
│   │       ├── state.rs          # AppState (shared storage, config, etc.)
│   │       ├── extractors/       # Custom Axum extractors
│   │       │   ├── mod.rs
│   │       │   ├── auth.rs       # AccessToken extractor + validation
│   │       │   ├── json.rs       # Matrix-compliant JSON extractor (proper error responses)
│   │       │   └── query.rs      # Query parameter extractors
│   │       ├── middleware/        # Tower middleware layers
│   │       │   ├── mod.rs
│   │       │   ├── rate_limit.rs
│   │       │   └── metrics.rs
│   │       └── handlers/         # CS API endpoint handlers
│   │           ├── mod.rs
│   │           ├── auth.rs       # Login, logout, token refresh
│   │           ├── register.rs   # Registration flows
│   │           ├── account.rs    # Whoami, deactivate, password change
│   │           ├── profile.rs    # Display name, avatar
│   │           ├── rooms.rs      # Create, join, leave, invite, ban, kick
│   │           ├── events.rs     # Send events, get events, relations
│   │           ├── sync.rs       # /sync and sliding sync
│   │           ├── state.rs      # Room state endpoints
│   │           ├── directory.rs  # Room directory
│   │           ├── typing.rs     # Typing notifications
│   │           ├── receipts.rs   # Read receipts
│   │           ├── presence.rs   # Presence
│   │           ├── search.rs     # Message search
│   │           ├── media.rs      # Upload/download proxy to maelstrom-media
│   │           ├── keys.rs       # E2EE key upload/query/claim
│   │           ├── to_device.rs  # To-device messaging
│   │           ├── threads.rs    # Thread endpoints
│   │           ├── relations.rs  # Relations (reactions, edits, etc.)
│   │           ├── capabilities.rs # Server capabilities
│   │           ├── versions.rs   # /_matrix/client/versions
│   │           └── wellknown.rs  # .well-known/matrix/client
│   ├── maelstrom-federation/     # Server-Server (S2S) API
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── router.rs         # Federation route tree
│   │       ├── client.rs         # Outbound federation HTTP client
│   │       ├── signing.rs        # Request signing (HTTP signatures)
│   │       ├── key_server.rs     # Key server endpoints and notary
│   │       ├── sender.rs         # Federation transaction sender (queue + retry)
│   │       ├── receiver.rs       # Inbound transaction processing
│   │       ├── backfill.rs       # Event backfill and gap filling
│   │       ├── state.rs          # State queries over federation
│   │       ├── joins.rs          # Remote join handling (make_join/send_join)
│   │       └── well_known.rs     # Server discovery (.well-known/matrix/server)
│   └── maelstrom-admin/          # Admin API and dashboard backend
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── router.rs         # Admin route tree
│           └── handlers/
│               ├── mod.rs
│               ├── users.rs      # User management (list, create, suspend, lock)
│               ├── rooms.rs      # Room management (list, shutdown, purge)
│               ├── media.rs      # Media management (retention enforcement, cleanup)
│               ├── federation.rs # Federation health, blocklists
│               ├── server.rs     # Server info, config, version
│               └── reports.rs    # Abuse reports management
├── src/
│   └── main.rs                   # Binary entry point: config loading, server startup
└── tests/                        # Integration tests (Rust test framework)
    ├── common/
    │   └── mod.rs                # Shared test helpers, fixtures, test server setup
    ├── auth_test.rs              # Login, logout, token validation
    ├── register_test.rs          # Registration flows
    ├── account_test.rs           # Whoami, password change, deactivation
    ├── profile_test.rs           # Display name, avatar URL
    ├── rooms_test.rs             # Room creation, joining, leaving, invites
    ├── events_test.rs            # Event sending and retrieval
    ├── sync_test.rs              # Sync endpoint, incremental sync
    ├── state_test.rs             # Room state endpoints
    ├── media_test.rs             # Media upload/download
    ├── keys_test.rs              # E2EE key management
    ├── federation_test.rs        # Federation transaction handling
    ├── typing_test.rs            # Typing notifications
    ├── receipts_test.rs          # Read receipts
    ├── directory_test.rs         # Room directory
    ├── admin_test.rs             # Admin API endpoints
    └── storage/                  # Storage layer integration tests
        ├── users_test.rs
        ├── rooms_test.rs
        ├── events_test.rs
        └── graph_test.rs         # Graph traversal and state resolution queries
```

### Key Architectural Principles

1. **Stateless application layer**: All Axum instances are interchangeable. No in-process state that can't be lost. Session data, caches, and coordination live in SurrealDB.

2. **Storage trait abstraction**: All database access goes through traits (`UserStore`, `RoomStore`, `EventStore`, `MediaMetadataStore`, etc.). The SurrealDB implementation is the primary backend; mock implementations enable testing without a database.

3. **Graph-first data model**: Matrix's event DAG, room membership, and relations are modeled as SurrealDB graph relations (`RELATE`). State resolution and timeline queries use graph traversal rather than joins.

4. **Horizontal scaling from day 1**: No singleton assumptions. Event ID generation uses content hashing (v4 format). Federation sender selection uses distributed locks via SurrealDB. No in-memory caches that assume single-instance.

5. **Zero-copy and CoW where practical**: Use `bytes::Bytes` for event bodies, `serde` zero-copy deserialization for read paths, and CoW (`Cow<'_, str>`) for string-heavy Matrix types.

### SurrealDB Graph Schema (Conceptual)

```
# Node tables
DEFINE TABLE user SCHEMAFULL;
DEFINE TABLE room SCHEMAFULL;
DEFINE TABLE event SCHEMAFULL;        # PDU storage
DEFINE TABLE device SCHEMAFULL;
DEFINE TABLE server_key SCHEMAFULL;

# Relation tables (graph edges)
DEFINE TABLE membership TYPE RELATION IN user OUT room;     # user->membership->room
DEFINE TABLE event_edge TYPE RELATION IN event OUT event;   # prev_events DAG
DEFINE TABLE state_event TYPE RELATION IN event OUT room;   # state snapshots
DEFINE TABLE sends TYPE RELATION IN user OUT event;         # who sent what
DEFINE TABLE reaction TYPE RELATION IN event OUT event;     # m.reaction
DEFINE TABLE thread TYPE RELATION IN event OUT event;       # m.thread
DEFINE TABLE redacts TYPE RELATION IN event OUT event;      # m.room.redaction
```

### Authentication Strategy

**Phase 1**: Traditional `m.login.password` flow via `/_matrix/client/v3/login`. Access tokens stored in SurrealDB, validated on every request via Axum extractor. This is what all current clients support.

**Phase 2 (later)**: OIDC-native authentication (MSC3861). Can coexist with password login — the spec allows advertising multiple flows via `GET /login`. This enables Element X and other Matrix 2.0 clients.

---

## Phase Breakdown

### Phase 1: Foundation
> Project skeleton, configuration, storage layer, Docker infrastructure

**Goal**: A compiling workspace with working SurrealDB connectivity, configuration system, and Docker Compose for the full clustered stack.

#### Tasks

- [ ] **1.1** Delete all existing `src/`, `schema/`, and old config files. Initialize fresh Cargo workspace (Rust 2024 edition)
- [ ] **1.2** Create workspace `Cargo.toml` with all crate members
- [ ] **1.3** Create crate skeletons: `maelstrom-core`, `maelstrom-storage`, `maelstrom-media`, `maelstrom-api`, `maelstrom-federation`, `maelstrom-admin`
- [ ] **1.4** Create `src/main.rs` binary entry point with Tokio runtime, config loading, and tracing initialization
- [ ] **1.5** Implement configuration system (`config/default.toml`): server bind address, hostname, SurrealDB connection, S3/RustFS endpoint, signing key paths, log level
- [ ] **1.6** `maelstrom-core`: Define `MatrixError` and `ErrorCode` enum (all standard Matrix error codes), implement `IntoResponse` for Axum
- [ ] **1.7** `maelstrom-core`: Define core types — event structs (PDU), room version enum, basic identifier re-exports from ruma
- [ ] **1.8** `maelstrom-storage`: Define storage traits (`UserStore`, `RoomStore`, `EventStore`, `DeviceStore`, `KeyStore`, `MediaMetadataStore`)
- [ ] **1.9** `maelstrom-storage`: Implement SurrealDB connection manager (connect, namespace/db setup, health check)
- [ ] **1.10** `maelstrom-storage`: Write SurrealQL schema definitions (DEFINE TABLE, DEFINE FIELD, DEFINE INDEX, DEFINE TABLE TYPE RELATION)
- [ ] **1.11** `maelstrom-storage`: Implement mock storage for testing
- [ ] **1.12** `maelstrom-api`: Set up Axum router skeleton with Tower middleware (CORS, tracing, compression, request ID)
- [ ] **1.13** `maelstrom-api`: Implement `AppState` (holds storage, config, signing keys)
- [ ] **1.14** `maelstrom-api`: Implement `/_matrix/client/versions` and `/.well-known/matrix/client`
- [ ] **1.15** `docker-compose.yml`: TiKV cluster (PD + 3 TiKV nodes) + SurrealDB (connected to TiKV) + RustFS
- [ ] **1.16** `docker-compose.dev.yml`: SurrealDB standalone (file-based) + RustFS single-node
- [ ] **1.17** `Dockerfile`: Multi-stage build for Maelstrom binary
- [ ] **1.18** CI pipeline: GitHub Actions for build, test, clippy, rustfmt
- [ ] **1.19** Write tests: storage connection, schema bootstrap, config loading, error serialization

#### Deliverable
Compiling workspace. `cargo test` passes. `docker compose up` brings up TiKV + SurrealDB + RustFS. Server starts and responds to `/versions`.

---

### Phase 2: User Authentication & Registration
> User accounts, device management, access tokens, login/logout

**Goal**: Users can register, log in, manage devices, and authenticate requests. All endpoints return spec-compliant responses.

**Matrix Spec Refs**: [Registration](https://spec.matrix.org/latest/client-server-api/#account-registration-and-management), [Login](https://spec.matrix.org/latest/client-server-api/#login), [Tokens](https://spec.matrix.org/latest/client-server-api/#using-access-tokens)

#### Tasks

- [ ] **2.1** `maelstrom-storage/surreal/users.rs`: Implement `UserStore` — create user, check exists, fetch by localpart, fetch password hash, store password hash (argon2)
- [ ] **2.2** `maelstrom-storage/surreal/devices.rs`: Implement `DeviceStore` — create device, remove device, remove all devices, list devices, generate access tokens
- [ ] **2.3** `maelstrom-api/extractors/auth.rs`: Access token extractor (from `Authorization: Bearer` header or `access_token` query param), validates against storage
- [ ] **2.4** `maelstrom-api/extractors/json.rs`: Matrix-compliant JSON body extractor (returns `M_NOT_JSON` / `M_BAD_JSON` errors)
- [ ] **2.5** `maelstrom-api/handlers/register.rs`: `POST /_matrix/client/v3/register` — username validation, password hashing (argon2), device creation, access token generation. Support `kind=guest` and `kind=user`. User-Interactive Authentication (UIA) flow with `m.login.dummy` stage
- [ ] **2.6** `maelstrom-api/handlers/register.rs`: `GET /_matrix/client/v3/register/available` — username availability check
- [ ] **2.7** `maelstrom-api/handlers/auth.rs`: `GET /_matrix/client/v3/login` — return supported login flows (`m.login.password`)
- [ ] **2.8** `maelstrom-api/handlers/auth.rs`: `POST /_matrix/client/v3/login` — validate credentials, create device + access token, return response with `user_id`, `access_token`, `device_id`
- [ ] **2.9** `maelstrom-api/handlers/auth.rs`: `POST /_matrix/client/v3/logout` and `/logout/all` — revoke access tokens, delete devices
- [ ] **2.10** `maelstrom-api/handlers/account.rs`: `GET /_matrix/client/v3/account/whoami` — return authenticated user's ID and device ID
- [ ] **2.11** `maelstrom-api/handlers/account.rs`: `POST /_matrix/client/v3/account/deactivate` — account deactivation with UIA
- [ ] **2.12** `maelstrom-api/handlers/account.rs`: `POST /_matrix/client/v3/account/password` — password change with UIA
- [ ] **2.13** `maelstrom-api/handlers/profile.rs`: `GET/PUT /_matrix/client/v3/profile/{userId}/displayname` and `/avatar_url`, `GET /profile/{userId}` (combined)
- [ ] **2.14** `maelstrom-api/middleware/rate_limit.rs`: Per-endpoint rate limiting using storage-backed counters (distributed-safe)
- [ ] **2.15** Write tests: `tests/auth_test.rs`, `tests/register_test.rs`, `tests/account_test.rs`, `tests/profile_test.rs`, `tests/storage/users_test.rs`

#### Deliverable
Users can register, log in with password, manage devices, view/update profiles. All token-authenticated endpoints work. Rate limiting is distributed-safe.

---

### Phase 3: Rooms & Membership
> Room creation, joining, leaving, inviting, banning, room directory

**Goal**: Users can create rooms, invite others, join/leave, and manage membership. Room state is stored as a graph in SurrealDB.

**Matrix Spec Refs**: [Room Creation](https://spec.matrix.org/latest/client-server-api/#creation), [Room Membership](https://spec.matrix.org/latest/client-server-api/#room-membership), [Room Directory](https://spec.matrix.org/latest/client-server-api/#listing-rooms)

#### Tasks

- [ ] **3.1** `maelstrom-core/events/pdu.rs`: Full PDU (Persistent Data Unit) structure — event_id (v4 content hash), room_id, sender, type, state_key, content, origin_server_ts, prev_events, auth_events, depth, signatures, hashes
- [ ] **3.2** `maelstrom-core/events/room.rs`: Room event content types — `m.room.create`, `m.room.member`, `m.room.power_levels`, `m.room.join_rules`, `m.room.name`, `m.room.topic`, `m.room.avatar`, `m.room.canonical_alias`, `m.room.history_visibility`, `m.room.guest_access`, `m.room.encryption`
- [ ] **3.3** `maelstrom-core/state/room_version.rs`: Room version definitions (v1-v12+), supported versions, default version
- [ ] **3.4** `maelstrom-core/signatures/`: Event content hashing (reference hash for event IDs), event signing with ed25519, signature verification
- [ ] **3.5** `maelstrom-storage/surreal/rooms.rs`: Implement `RoomStore` — create room record, store room state, membership graph operations using RELATE (user->membership->room with membership state: join/invite/leave/ban/knock)
- [ ] **3.6** `maelstrom-storage/surreal/events.rs`: Implement `EventStore` — store PDU, build prev_events links using RELATE (event_edge), query timeline (forward/backward pagination), query by event_id
- [ ] **3.7** `maelstrom-storage/surreal/state.rs`: Room state storage — current state map (type, state_key) -> event_id, state snapshots at event positions
- [ ] **3.8** `maelstrom-core/state/v2.rs`: State resolution algorithm v2 — implement the algorithm from the spec for resolving conflicting state (needed for room versions 2+)
- [ ] **3.9** `maelstrom-api/handlers/rooms.rs`: `POST /_matrix/client/v3/createRoom` — create room with initial state events (create, member, power_levels, join_rules, etc.), return room_id
- [ ] **3.10** `maelstrom-api/handlers/rooms.rs`: `POST /_matrix/client/v3/join/{roomIdOrAlias}`, `POST /rooms/{roomId}/join` — join a room
- [ ] **3.11** `maelstrom-api/handlers/rooms.rs`: `POST /rooms/{roomId}/invite` — invite a user
- [ ] **3.12** `maelstrom-api/handlers/rooms.rs`: `POST /rooms/{roomId}/leave`, `/kick`, `/ban`, `/unban` — membership state changes
- [ ] **3.13** `maelstrom-api/handlers/rooms.rs`: `GET /joined_rooms` — list rooms the user has joined (graph query: user->membership[state=join]->room)
- [ ] **3.14** `maelstrom-api/handlers/state.rs`: `GET/PUT /rooms/{roomId}/state/{eventType}/{stateKey}` — get/set room state events
- [ ] **3.15** `maelstrom-api/handlers/directory.rs`: `GET/PUT /_matrix/client/v3/directory/room/{roomAlias}` — room alias management
- [ ] **3.16** `maelstrom-api/handlers/directory.rs`: `GET /publicRooms` — public room directory with pagination and filtering
- [ ] **3.17** Authorization checking: verify power levels, join rules, membership state before allowing actions
- [ ] **3.18** Write tests: `tests/rooms_test.rs`, `tests/state_test.rs`, `tests/directory_test.rs`, `tests/storage/rooms_test.rs`, `tests/storage/events_test.rs`, `tests/storage/graph_test.rs`

#### Deliverable
Full room lifecycle works. Membership graph queries are performant. State resolution produces correct results. Authorization is enforced.

---

### Phase 4: Messaging & Sync
> Sending/receiving messages, sync endpoint, typing, receipts, presence

**Goal**: Users can send messages, receive them via `/sync`, and see typing indicators and read receipts. This is where the homeserver becomes usable with real clients.

**Matrix Spec Refs**: [Sending Events](https://spec.matrix.org/latest/client-server-api/#sending-events-to-a-room), [Syncing](https://spec.matrix.org/latest/client-server-api/#syncing), [Sliding Sync](https://spec.matrix.org/latest/client-server-api/#sliding-sync)

#### Tasks

- [ ] **4.1** `maelstrom-api/handlers/events.rs`: `PUT /rooms/{roomId}/send/{eventType}/{txnId}` — send message events (m.room.message, etc.) with transaction ID idempotency
- [ ] **4.2** `maelstrom-api/handlers/events.rs`: `GET /rooms/{roomId}/event/{eventId}` — get single event
- [ ] **4.3** `maelstrom-api/handlers/events.rs`: `GET /rooms/{roomId}/messages` — paginated message history (forward/backward, using `from`/`to` tokens)
- [ ] **4.4** `maelstrom-api/handlers/events.rs`: `PUT /rooms/{roomId}/redact/{eventId}/{txnId}` — redact events
- [ ] **4.5** `maelstrom-storage/surreal/sync.rs`: Sync token generation and tracking — stream position per user/device, efficient "what's changed since" queries
- [ ] **4.6** `maelstrom-api/handlers/sync.rs`: `GET /_matrix/client/v3/sync` — full sync endpoint with long-polling. Returns: joined rooms (timeline, state, ephemeral), invited rooms, left rooms. Incremental sync via `since` token
- [ ] **4.7** `maelstrom-api/handlers/sync.rs`: Sliding Sync (`POST /_matrix/client/v3/sync` with request body) — room list sorting, ranges, required_state, timeline_limit, extensions
- [ ] **4.8** `maelstrom-api/handlers/typing.rs`: `PUT /rooms/{roomId}/typing/{userId}` — typing notification (ephemeral, stored in SurrealDB with TTL or distributed cache)
- [ ] **4.9** `maelstrom-api/handlers/receipts.rs`: `POST /rooms/{roomId}/receipt/{receiptType}/{eventId}` — read receipts
- [ ] **4.10** `maelstrom-api/handlers/presence.rs`: `GET/PUT /presence/{userId}/status` — presence (online/offline/unavailable)
- [ ] **4.11** Transaction ID idempotency: store txnId->event_id mappings per device, return existing event on duplicate sends
- [ ] **4.12** `maelstrom-api/handlers/search.rs`: `POST /_matrix/client/v3/search` — full-text message search (leverage SurrealDB full-text indexing)
- [ ] **4.13** Write tests: `tests/events_test.rs`, `tests/sync_test.rs`, `tests/typing_test.rs`, `tests/receipts_test.rs`

#### Deliverable
A real Matrix client (Element) can connect, sync rooms, send/receive messages, see typing indicators and read receipts. Sliding Sync works for Element X.

---

### Phase 5: End-to-End Encryption (E2EE)
> Key upload, key query, key claim, to-device messages

**Goal**: Clients can perform key exchange and send encrypted messages.

**Matrix Spec Refs**: [E2EE](https://spec.matrix.org/latest/client-server-api/#end-to-end-encryption)

#### Tasks

- [ ] **5.1** `maelstrom-storage/surreal/keys.rs`: Implement `KeyStore` — store device keys (ed25519, curve25519), one-time keys (OTKs), fallback keys, cross-signing keys
- [ ] **5.2** `maelstrom-api/handlers/keys.rs`: `POST /_matrix/client/v3/keys/upload` — upload device keys and OTKs
- [ ] **5.3** `maelstrom-api/handlers/keys.rs`: `POST /_matrix/client/v3/keys/query` — query device keys for users
- [ ] **5.4** `maelstrom-api/handlers/keys.rs`: `POST /_matrix/client/v3/keys/claim` — claim one-time keys for session establishment
- [ ] **5.5** `maelstrom-api/handlers/keys.rs`: `POST /_matrix/client/v3/keys/changes` — key change tracking since a sync token
- [ ] **5.6** `maelstrom-api/handlers/keys.rs`: Cross-signing key upload and signatures (`POST /keys/device_signing/upload`, `POST /keys/signatures/upload`)
- [ ] **5.7** `maelstrom-api/handlers/to_device.rs`: `PUT /sendToDevice/{eventType}/{txnId}` — send to-device messages (key requests, key forwards, verification)
- [ ] **5.8** Sync integration: include to-device messages and device list changes in `/sync` response
- [ ] **5.9** Write tests: `tests/keys_test.rs`

#### Deliverable
Encrypted messaging works end-to-end with Element. Key verification flows succeed.

---

### Phase 6: Media
> Upload, download, thumbnails, URL previews, retention policy

**Goal**: Users can upload/download files, images render with thumbnails, and admins can configure retention policies.

**Matrix Spec Refs**: [Content Repository](https://spec.matrix.org/latest/client-server-api/#content-repository)

#### Tasks

- [ ] **6.1** `maelstrom-media/client.rs`: S3 client wrapper using `aws-sdk-s3` — connect to RustFS, upload, download, delete, head
- [ ] **6.2** `maelstrom-media/upload.rs`: Upload handling — content-type validation, size limits, generate MXC URI, store blob in RustFS + metadata in SurrealDB
- [ ] **6.3** `maelstrom-media/download.rs`: Download handling — resolve MXC URI to S3 key, stream response, support range requests
- [ ] **6.4** `maelstrom-media/thumbnail.rs`: Thumbnail generation — resize images on demand or pre-generate, cache in RustFS
- [ ] **6.5** `maelstrom-api/handlers/media.rs`: `POST /_matrix/media/v3/upload`, `GET /_matrix/media/v3/download/{serverName}/{mediaId}`, `GET /_matrix/media/v3/thumbnail/{serverName}/{mediaId}`
- [ ] **6.6** `maelstrom-api/handlers/media.rs`: `GET /_matrix/media/v3/preview_url` — URL preview with OpenGraph metadata fetching
- [ ] **6.7** `maelstrom-api/handlers/media.rs`: `GET /_matrix/media/v3/config` — upload size limits
- [ ] **6.8** `maelstrom-media/retention.rs`: Retention policy engine — configurable max age, max size per user, media type filters, quarantine support
- [ ] **6.9** `maelstrom-storage/surreal/media.rs`: Media metadata storage — MXC URI mapping, upload timestamps, file sizes, content types, quarantine status
- [ ] **6.10** Write tests: `tests/media_test.rs`

#### Deliverable
File uploads/downloads work. Images show thumbnails in clients. Retention policies can be configured and enforced.

---

### Phase 7: Federation
> Server-to-server API, event signing, key management, remote joins, transaction processing

**Goal**: Maelstrom can federate with other Matrix homeservers (Synapse, Dendrite, Conduwuit). Users on different servers can communicate.

**Matrix Spec Refs**: [Server-Server API](https://spec.matrix.org/latest/server-server-api/)

#### Tasks

- [ ] **7.1** `maelstrom-core/signatures/keys.rs`: Ed25519 signing key generation, storage, and rotation. Key ID format: `ed25519:key_id`
- [ ] **7.2** `maelstrom-federation/signing.rs`: HTTP request signing — sign outbound requests per spec (Authorization header with `X-Matrix` scheme)
- [ ] **7.3** `maelstrom-federation/key_server.rs`: `GET /_matrix/key/v2/server` — serve own signing keys. `GET /_matrix/key/v2/query/{serverName}` — notary queries
- [ ] **7.4** `maelstrom-federation/client.rs`: Outbound federation HTTP client — server discovery (`.well-known/matrix/server`, SRV records), TLS validation, request signing, retries
- [ ] **7.5** `maelstrom-federation/receiver.rs`: `PUT /_matrix/federation/v1/send/{txnId}` — receive and process inbound transactions (PDUs + EDUs)
- [ ] **7.6** `maelstrom-federation/receiver.rs`: PDU validation — check signatures, verify auth events, check against auth rules, perform state resolution if needed
- [ ] **7.7** `maelstrom-federation/joins.rs`: `GET /make_join/{roomId}/{userId}`, `PUT /send_join/{roomId}/{eventId}` — remote join protocol
- [ ] **7.8** `maelstrom-federation/joins.rs`: `GET /make_leave/{roomId}/{userId}`, `PUT /send_leave/{roomId}/{eventId}` — remote leave
- [ ] **7.9** `maelstrom-federation/backfill.rs`: `GET /backfill/{roomId}`, `GET /get_missing_events/{roomId}` — fill timeline gaps
- [ ] **7.10** `maelstrom-federation/state.rs`: `GET /state/{roomId}`, `GET /state_ids/{roomId}`, `GET /event/{eventId}` — query remote room state
- [ ] **7.11** `maelstrom-federation/sender.rs`: Transaction sender — queue outbound PDUs/EDUs per destination, batch into transactions, retry with backoff, distributed queue (no single sender assumption)
- [ ] **7.12** Federation EDU handling: typing notifications, device list updates, presence, receipts over federation
- [ ] **7.13** Media over federation: proxy downloads from remote MXC URIs
- [ ] **7.14** E2EE over federation: device key queries for remote users (`GET /user/keys/query`)
- [ ] **7.15** Write tests: `tests/federation_test.rs`

#### Deliverable
Two Maelstrom instances can federate. Maelstrom can federate with Synapse. Remote joins, messaging, and backfill work correctly.

---

### Phase 8: Matrix 2.0+ Features
> Threads, relations, reactions, polls, spaces, knocking, restricted rooms, account suspension

**Goal**: Full Matrix 2.0+ feature coverage for modern client compatibility.

**Matrix Spec Refs**: [Relations](https://spec.matrix.org/latest/client-server-api/#forming-relationships-between-events), [Threads](https://spec.matrix.org/latest/client-server-api/#threading), [Spaces](https://spec.matrix.org/latest/client-server-api/#spaces), [Knocking](https://spec.matrix.org/latest/client-server-api/#knocking-on-rooms)

#### Tasks

- [ ] **8.1** `maelstrom-api/handlers/relations.rs`: `GET /rooms/{roomId}/relations/{eventId}` — fetch relations (reactions, edits, threads, references) with pagination and filtering by rel_type/event_type
- [ ] **8.2** Reaction aggregation: bundle reaction counts in sync and event responses
- [ ] **8.3** Event editing: `m.replace` relation handling, serve edited content
- [ ] **8.4** `maelstrom-api/handlers/threads.rs`: `GET /rooms/{roomId}/threads` — thread listing with pagination
- [ ] **8.5** Thread-aware sync: include `m.thread` relation in sync, thread notification counts
- [ ] **8.6** Spaces support: `m.space.child` / `m.space.parent` state events, `GET /rooms/{roomId}/hierarchy` — space hierarchy traversal (graph query)
- [ ] **8.7** Knocking: `POST /knock/{roomIdOrAlias}` — knock on a room, membership state `knock`
- [ ] **8.8** Restricted rooms: `m.room.join_rules` with `restricted` type, verify membership in allowed rooms
- [ ] **8.9** Polls: `m.poll.start`, `m.poll.response`, `m.poll.end` event handling
- [ ] **8.10** Account locking and suspension (MSC3939): `locked` and `suspended` account states, reject requests from locked/suspended accounts
- [ ] **8.11** Improved reporting: `POST /rooms/{roomId}/report/{eventId}` — enhanced abuse reporting
- [ ] **8.12** Invite blocking and policy servers: `m.policy.rule.*` state events, server-side enforcement
- [ ] **8.13** Write tests for all above

#### Deliverable
All Matrix 2.0+ features work. Element X and Element Web have full feature parity.

---

### Phase 9: Admin API & Operations
> Admin dashboard backend, moderation tools, metrics, media retention, Synapse migration

**Goal**: Production-ready operations tooling.

#### Tasks

- [ ] **9.1** `maelstrom-admin/handlers/users.rs`: List users, create users, deactivate/reactivate, reset password, make admin, view sessions/devices, suspend/lock
- [ ] **9.2** `maelstrom-admin/handlers/rooms.rs`: List rooms, room details, shutdown room (remove all members, block rejoin), purge room history, set room admin
- [ ] **9.3** `maelstrom-admin/handlers/media.rs`: Media usage stats, quarantine media, purge media by user/room/age, trigger retention policy sweep, view/edit retention config
- [ ] **9.4** `maelstrom-admin/handlers/federation.rs`: Federation stats (queue depth, destination health), blocklist management, force retry stuck destinations
- [ ] **9.5** `maelstrom-admin/handlers/server.rs`: Server version, config view (non-sensitive), database stats, connected users count, uptime
- [ ] **9.6** `maelstrom-admin/handlers/reports.rs`: List abuse reports, view report details, take action (redact, kick, ban)
- [ ] **9.7** Prometheus metrics endpoint: request latency, request count by endpoint, active sync connections, federation queue depth, database query latency, media storage usage
- [ ] **9.8** Health check endpoints: `/_health/live`, `/_health/ready` (checks SurrealDB + RustFS connectivity)
- [ ] **9.9** Structured logging with request correlation IDs (already in Phase 1 middleware, refine here)
- [ ] **9.10** Admin authentication: separate admin tokens or admin flag on user accounts
- [ ] **9.11** Write tests: `tests/admin_test.rs`

#### Deliverable
Full admin API for managing users, rooms, media, federation. Prometheus metrics exported. Health checks work for Kubernetes probes.

---

### Phase 10: Complement Testing & Hardening
> Full Complement pass, client compatibility testing, performance tuning

**Goal**: 100% Complement pass rate. Validated with real clients. Performance meets targets.

#### Tasks

- [ ] **10.1** Create Complement-compatible Dockerfile: expose 8008 (CS API) and 8448 (Federation HTTPS), accept Complement env vars for server name and TLS certs, include HEALTHCHECK
- [ ] **10.2** Set up Complement CI pipeline: run `COMPLEMENT_BASE_IMAGE=maelstrom:test go test ./tests/...` in GitHub Actions
- [ ] **10.3** Triage and fix all Complement test failures — iterate until 100% pass
- [ ] **10.4** Client compatibility testing: connect Element Web, Element X (Android/iOS), FluffyChat, nheko. Verify core flows work
- [ ] **10.5** Performance benchmarking: large room joins (10k+ members), high message throughput, sync latency under load
- [ ] **10.6** Horizontal scaling validation: run 3+ Maelstrom instances behind load balancer, verify consistency and no split-brain
- [ ] **10.7** Chaos testing: kill nodes during operations, verify recovery
- [ ] **10.8** Security audit: review all input validation, auth checks, rate limiting. OWASP top 10 review.
- [ ] **10.9** Documentation: deployment guide, configuration reference, architecture overview

#### Deliverable
Production-ready release candidate. 100% Complement. Validated with clients. Performance targets met.

---

### Phase 11: Synapse Migration & 1.0
> Migration tooling, production hardening, 1.0 release

#### Tasks

- [ ] **11.1** Synapse database migration tool: read Synapse PostgreSQL schema, convert and import users, rooms, events, media metadata into SurrealDB
- [ ] **11.2** Media migration: copy media files from Synapse's media store to RustFS
- [ ] **11.3** Signing key migration: import Synapse's ed25519 signing keys to maintain federation identity
- [ ] **11.4** Validation suite: compare migrated data against Synapse for correctness
- [ ] **11.5** OIDC-native authentication (MSC3861): implement as alternative login flow, enabling full Matrix 2.0 auth
- [ ] **11.6** Helm chart for Kubernetes deployment: SurrealDB (TiKV), RustFS, Maelstrom replicas, ingress, TLS
- [ ] **11.7** Final security review and penetration testing
- [ ] **11.8** 1.0 release

#### Deliverable
Synapse users can migrate to Maelstrom. Kubernetes deployment is one-command. 1.0 released.

---

## Phase Tracking

| Phase | Name | Status | Dependencies |
|-------|------|--------|-------------|
| 1 | Foundation | Not Started | — |
| 2 | Authentication & Registration | Not Started | Phase 1 |
| 3 | Rooms & Membership | Not Started | Phase 2 |
| 4 | Messaging & Sync | Not Started | Phase 3 |
| 5 | E2EE | Not Started | Phase 4 |
| 6 | Media | Not Started | Phase 1 (can parallel with 3-5) |
| 7 | Federation | Not Started | Phase 4 |
| 8 | Matrix 2.0+ Features | Not Started | Phase 4 |
| 9 | Admin & Operations | Not Started | Phase 6, 7 |
| 10 | Complement & Hardening | Not Started | Phase 7, 8 |
| 11 | Migration & 1.0 | Not Started | Phase 10 |

### Parallelization Opportunities

```
Phase 1 ──> Phase 2 ──> Phase 3 ──> Phase 4 ──> Phase 5
                                       │  ╲
                                       │   ╲──> Phase 7 ──> Phase 10 ──> Phase 11
                                       │                        ↑
Phase 1 ──> Phase 6 (media, parallel) ─┘   Phase 8 ───────────┘
                                                ↑
                                           Phase 4 ──> Phase 9
```

- **Phase 6 (Media)** can start after Phase 1, runs parallel with Phases 3-5
- **Phase 8 (Matrix 2.0+)** can start after Phase 4, runs parallel with Phase 7
- **Phase 9 (Admin)** can start after Phase 4, runs parallel with Phase 7-8

---

## Testing Strategy

### Test Locations & Conventions

All tests live in the `tests/` directory (not inline in source). Each test file maps to a functional area.

### Test Layers

1. **Unit tests** (`tests/storage/`): Test storage trait implementations against SurrealDB (file-based for CI) and mock storage. Verify graph queries, CRUD operations, schema integrity.

2. **API integration tests** (`tests/*.rs`): Spin up a full Axum server with test storage. Make HTTP requests and verify responses match the Matrix spec exactly — status codes, JSON structure, error codes, headers.

3. **Multi-instance tests**: Verify that two Maelstrom instances sharing the same SurrealDB produce correct behavior (no race conditions, consistent state).

4. **Complement tests** (Phase 10): Black-box spec compliance. The gold standard.

5. **Client smoke tests** (Phase 10): Manual and scripted tests with real Matrix clients.

### Test Infrastructure

- `tests/common/mod.rs`: Shared test setup — start test server, create test users, helper functions for common operations (register, login, create room, send message, sync)
- Tests use SurrealDB file-based mode (no TiKV needed for CI)
- Docker Compose for tests needing full stack (federation, media, clustering)
- GitHub Actions runs `cargo test` on every push/PR

### Test Naming Convention

```
tests/
  auth_test.rs          → test_login_password_success, test_login_invalid_password, ...
  register_test.rs      → test_register_new_user, test_register_username_taken, ...
  rooms_test.rs         → test_create_room_default, test_join_public_room, ...
  storage/
    users_test.rs       → test_create_user, test_fetch_user_by_localpart, ...
    graph_test.rs       → test_membership_graph_traversal, test_event_dag_query, ...
```

---

## Key Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|-----------|
| SurrealDB v3 stability (alpha/early release) | Data loss, query bugs | Pin exact version, comprehensive test coverage, file bugs upstream, keep storage trait abstraction so backend can be swapped |
| RustFS maturity (alpha) | Media reliability | S3 interface means MinIO is a drop-in fallback. Abstract behind aws-sdk-s3 |
| Matrix spec complexity | Missed edge cases | Complement tests catch spec violations. Start with core flows, expand incrementally |
| State resolution correctness | Federation breakage | Port well-tested algorithms from Synapse/Conduwuit. Extensive property-based testing |
| Horizontal scaling edge cases | Data inconsistency | Test multi-instance scenarios from Phase 1. SurrealDB+TiKV provides strong consistency |
| Complement test count | Long CI times | Parallelize test runs, use fast SurrealDB mode for tests |

---

## Reference Links

- [Matrix Spec (latest)](https://spec.matrix.org/latest/)
- [Client-Server API](https://spec.matrix.org/latest/client-server-api/)
- [Server-Server API](https://spec.matrix.org/latest/server-server-api/)
- [Room Versions](https://spec.matrix.org/unstable/rooms/)
- [Complement Test Suite](https://github.com/matrix-org/complement)
- [SurrealDB Docs](https://surrealdb.com/docs)
- [RustFS GitHub](https://github.com/rustfs/rustfs)
- [Axum Docs](https://docs.rs/axum/latest/axum/)
- [Ruma (Matrix types)](https://docs.rs/ruma/latest/ruma/)
