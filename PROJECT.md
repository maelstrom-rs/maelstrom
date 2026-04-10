# Maelstrom Project Plan

> Enterprise-Grade Clustered Matrix Homeserver — Complete Rewrite
> Last updated: 2026-04-10

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
| Matrix Types | maelstrom-core | — | Matrix identifiers, canonical JSON, event signing (no ruma dependency) |
| Async Runtime | Tokio | 1.x | Async I/O |
| Logging | tracing / tracing-subscriber | 0.1.x / 0.3.x | Structured logging + OpenTelemetry |
| Metrics | metrics / metrics-exporter-prometheus | 0.x | Prometheus-compatible metrics |
| Testing | Rust test framework + Complement | — | Unit/integration in `tests/`, black-box via Complement |
| Deployment | Docker, Docker Compose, Helm | — | Container orchestration |

### Workspace Structure

```
maelstrom/
├── Cargo.toml                    # Workspace root
├── Makefile                      # Dev workflow targets (make help)
├── PROJECT.md                    # This file
├── maelstrom-product-spec.md     # Product specification
├── docker-compose.yml            # TiKV cluster + RustFS + SurrealDB for dev/test
├── docker-compose.dev.yml        # Lightweight single-node dev setup
├── Dockerfile                    # Maelstrom server image (also used by Complement)
├── config/
│   └── example.toml              # Documented example config (cp to local.toml)
├── db/
│   └── schema.surql              # SurrealDB schema (single source of truth)
├── crates/
│   ├── maelstrom-core/           # Matrix protocol types and domain model
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # pub mod matrix (single module — everything is Matrix-specific)
│   │       └── matrix/
│   │           ├── mod.rs        # Module declarations
│   │           ├── error.rs      # MatrixError, ErrorCode enum, IntoResponse for Axum
│   │           ├── id.rs         # UserId, RoomId, EventId, ServerName, DeviceId, RoomAlias (validated newtypes)
│   │           ├── event.rs      # Pdu, ClientEvent, StrippedEvent + generate_event_id/room_id, timestamp_ms
│   │           ├── content.rs    # Typed event content: Create, Member, PowerLevels, JoinRules, etc.
│   │           ├── room.rs       # Membership, JoinRule, HistoryVisibility, RoomVersion, PowerLevelContent, event_type constants
│   │           ├── edu.rs        # Typed EDUs: Typing, Presence, Receipt, DeviceListUpdate
│   │           ├── json.rs       # CanonicalJson — deterministic serialization for signing
│   │           ├── keys.rs       # Ed25519 KeyPair generation, signing, verification
│   │           ├── signing.rs    # Event signing, hashing, verification (uses CanonicalJson + KeyPair)
│   │           ├── state.rs      # State Resolution v2 algorithm
│   │           └── ephemeral.rs  # EphemeralStore — in-memory typing/presence (DashMap + gossip)
│   ├── maelstrom-storage/        # Storage abstraction + SurrealDB implementation
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── traits.rs         # Storage trait definitions (UserStore, RoomStore, EventStore, etc.)
│   │       ├── surreal/          # SurrealDB implementation
│   │       │   ├── mod.rs
│   │       │   ├── connection.rs # Connection pool, namespace/db setup
│   │       │   ├── schema.rs     # Loads db/schema.surql via include_str!()
│   │       │   ├── users.rs      # User CRUD operations
│   │       │   ├── rooms.rs      # Room CRUD, membership graph queries
│   │       │   ├── events.rs     # Event storage, DAG traversal, timeline queries
│   │       │   ├── devices.rs    # Device and access token management
│   │       │   ├── keys.rs       # E2EE key storage (device keys, OTKs, cross-signing)
│   │       │   ├── media.rs      # Media metadata (blob refs to RustFS)
│   │       │   ├── account_data.rs # Per-user and per-room account data
│   │       │   ├── receipts.rs   # Read receipt storage
│   │       │   ├── relations.rs  # Event relation storage (reactions, threads, edits)
│   │       │   └── federation.rs # Remote server keys, federation destinations
│   │       └── mock.rs           # Mock storage for unit tests
│   ├── maelstrom-media/          # Media handling via S3 (RustFS)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── client.rs         # S3 client wrapper (aws-sdk-s3)
│   │       ├── preview.rs        # URL preview / Open Graph metadata extraction
│   │       ├── thumbnail.rs      # Thumbnail generation (image crate)
│   │       └── retention.rs      # Retention policy engine (background task)
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
│   │           ├── util.rs       # Shared helpers: password hashing, percent encoding, membership check
│   │           ├── auth.rs       # Login, logout, token refresh
│   │           ├── register.rs   # Registration flows
│   │           ├── account.rs    # Whoami, deactivate, password change
│   │           ├── profile.rs    # Display name, avatar
│   │           ├── rooms.rs      # Create, join, leave, invite, ban, kick, upgrade
│   │           ├── events.rs     # Send events, get events, state, messages, redact
│   │           ├── sync.rs       # /sync (long-poll) and sliding sync
│   │           ├── directory.rs  # Room directory and aliases
│   │           ├── typing.rs     # Typing indicators
│   │           ├── receipts.rs   # Read receipts
│   │           ├── presence.rs   # User presence
│   │           ├── search.rs     # Server-side message search
│   │           ├── media.rs      # Upload/download proxy to maelstrom-media
│   │           ├── keys.rs       # E2EE key upload/query/claim
│   │           ├── to_device.rs  # Send-to-device messaging
│   │           ├── threads.rs    # Thread listing
│   │           ├── relations.rs  # Relations (reactions, edits, aggregations)
│   │           ├── knock.rs      # Room knocking
│   │           ├── spaces.rs     # Space hierarchy
│   │           ├── reporting.rs  # Content reporting
│   │           ├── capabilities.rs # Server capabilities and push rules
│   │           ├── versions.rs   # /_matrix/client/versions
│   │           ├── wellknown.rs  # .well-known/matrix/client
│   │           └── health.rs     # Health check endpoints
│   ├── maelstrom-federation/     # Server-Server (S2S) API
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── router.rs         # Federation route tree
│   │       ├── client.rs         # Outbound HTTP client with discovery (DashMap cache)
│   │       ├── signing.rs        # X-Matrix request signing
│   │       ├── key_server.rs     # Signing key distribution endpoints
│   │       ├── sender.rs         # Transaction sender with DashMap queues + backoff
│   │       ├── receiver.rs       # Inbound transaction processing
│   │       ├── backfill.rs       # Event backfill and missing events
│   │       ├── state.rs          # Room state and state_ids endpoints
│   │       ├── joins.rs          # make_join/send_join, make_leave/send_leave
│   │       ├── invite.rs         # Federation invite flow
│   │       ├── queries.rs        # Profile and directory queries
│   │       └── user_keys.rs      # Cross-server device key queries
│   └── maelstrom-admin/          # Admin API and SSR dashboard
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── router.rs         # Admin route tree + static file serving
│           ├── auth.rs           # Admin authentication (is_admin check)
│           ├── templates.rs      # Askama HTML templates for dashboard
│           └── handlers/
│               ├── mod.rs
│               ├── dashboard.rs  # Dashboard overview
│               ├── users.rs      # User management
│               ├── rooms.rs      # Room management
│               ├── media.rs      # Media management and quarantine
│               ├── federation.rs # Federation status
│               ├── server.rs     # Server info, metrics, health
│               └── reports.rs    # Content report review
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

### Type System Design

Maelstrom uses a custom Matrix type system (no ruma dependency) designed for homeserver performance:

**Single module: `maelstrom_core::matrix`** — everything is Matrix-specific:

```
┌──────────────────────────────────────────────┐
│  Handlers (CS API, Federation, Admin)        │  ← uses matrix::* types
├──────────────────────────────────────────────┤
│  matrix::event     Pdu, ClientEvent,         │  ← domain model with
│  matrix::content   MemberContent, PowerLevels│     signing methods
│  matrix::room      RoomVersion, JoinRule,    │     and typed content
│  matrix::edu       TypingEdu, PresenceEdu    │
│  matrix::error     MatrixError, ErrorCode    │
│  matrix::ephemeral EphemeralStore (DashMap)   │
├──────────────────────────────────────────────┤
│  matrix::id        UserId, RoomId, ServerName│  ← validated identifiers
├──────────────────────────────────────────────┤
│  matrix::json      CanonicalJson             │  ← deterministic serialization
│  matrix::keys      Ed25519 KeyPair           │     for event signing
│  matrix::signing   sign_event, verify, hash  │
│  matrix::state     State Resolution v2       │
└──────────────────────────────────────────────┘
```

**Key design decisions (inspired by ruma, tailored for a homeserver):**

| Decision | Rationale |
|----------|-----------|
| `CanonicalJson` is protocol-agnostic | Generic deterministic JSON — no "remove signatures" knowledge |
| `Pdu` owns signing methods | `.sign()`, `.content_hash()` know which fields to strip |
| `Content::parse()` is lazy | Content stored as `serde_json::Value`, typed only when accessed |
| `Content` enum has `Raw` fallback | Unknown/custom event types pass through without loss |
| `event_type::*` are `&str` constants, not enum | Custom event types (`com.example.foo`) must work — can't enumerate all |
| `Membership`, `JoinRule`, etc. are enums | Finite set per spec, exhaustive `match` catches missing cases |
| `PowerLevelContent` parses once | Replaces repeated `.get("users").and_then()` chains across handlers |
| `ClientEvent` is a separate type from `Pdu` | Different fields — no `Option` bloat on fields that are always/never present |
| `RoomVersion` enum drives behavior | `.supports_knock()`, `.event_id_format()` etc. — no scattered string matching |
| No borrowed/owned ID pairs (yet) | Simpler than ruma's `&UserId`/`OwnedUserId` split — defer to optimization phase |

**What we deliberately don't adopt from ruma:**
- Macro-generated endpoint types (we're always the server, not a client library)
- Incoming/Outgoing request distinction (unnecessary for a homeserver)
- Heavy generic hierarchies (`OriginalSyncStateEvent<C>`) — we use concrete types
- Exhaustive event content enums for every MSC — `Content::Raw` handles unknowns

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

- [x] **1.1** Delete all existing `src/`, `schema/`, and old config files. Initialize fresh Cargo workspace (Rust 2024 edition)
- [x] **1.2** Create workspace `Cargo.toml` with all crate members
- [x] **1.3** Create crate skeletons: `maelstrom-core`, `maelstrom-storage`, `maelstrom-media`, `maelstrom-api`, `maelstrom-federation`, `maelstrom-admin`
- [x] **1.4** Create `src/main.rs` binary entry point with Tokio runtime, config loading, and tracing initialization
- [x] **1.5** Implement configuration system (`config/example.toml`): server bind address, hostname, SurrealDB connection, S3/RustFS endpoint
- [x] **1.6** `maelstrom-core`: Define `MatrixError` and `ErrorCode` enum (all standard Matrix error codes), implement `IntoResponse` for Axum
- [x] **1.7** `maelstrom-core`: Define core types — Matrix identifier newtypes (UserId, RoomId, EventId, DeviceId, ServerName, RoomAlias) with parsing, validation, serde
- [x] **1.8** `maelstrom-storage`: Define storage traits (`UserStore`, `DeviceStore`, `HealthCheck`, `Storage`)
- [x] **1.9** `maelstrom-storage`: Implement SurrealDB connection manager (connect, namespace/db setup, health check)
- [x] **1.10** `maelstrom-storage`: SurrealQL schema in external `db/schema.surql`, loaded via `include_str!()`
- [x] **1.11** `maelstrom-storage`: Implement mock storage for testing
- [x] **1.12** `maelstrom-api`: Set up Axum router skeleton with Tower middleware (CORS, tracing, compression)
- [x] **1.13** `maelstrom-api`: Implement `AppState` (holds storage, config, server name)
- [x] **1.14** `maelstrom-api`: Implement `/_matrix/client/versions`, `/.well-known/matrix/client`, `/_health/live`, `/_health/ready`
- [x] **1.15** `docker-compose.yml`: TiKV cluster (PD + 3 TiKV nodes) + SurrealDB (connected to TiKV) + RustFS
- [x] **1.16** `docker-compose.dev.yml`: SurrealDB standalone (SurrealKV) + RustFS single-node
- [x] **1.17** `Dockerfile`: Multi-stage build for Maelstrom binary with dependency caching
- [x] **1.18** CI pipeline: GitHub Actions for fmt, clippy, build, test
- [x] **1.19** Write tests: error serialization (4), identifier parsing (9), mock storage CRUD (12), API endpoints (5) — 30 tests total
- [x] **1.20** `Makefile`: Dev workflow targets (build, test, dev-up/down, db-init/drop/shell, stack-up/down)

#### Deliverable — Complete
Compiling workspace with 0 warnings. 30 tests pass. Docker Compose brings up TiKV + SurrealDB + RustFS. Server responds to `/versions`, `/.well-known`, `/_health/*`. Makefile provides dev workflow.

---

### Phase 2: User Authentication & Registration
> User accounts, device management, access tokens, login/logout

**Goal**: Users can register, log in, manage devices, and authenticate requests. All endpoints return spec-compliant responses.

**Matrix Spec Refs**: [Registration](https://spec.matrix.org/latest/client-server-api/#account-registration-and-management), [Login](https://spec.matrix.org/latest/client-server-api/#login), [Tokens](https://spec.matrix.org/latest/client-server-api/#using-access-tokens)

#### Tasks

- [x] **2.1** `maelstrom-storage/surreal/users.rs`: Implement `UserStore` — create user + profile, check exists, fetch by localpart, set password hash, set deactivated, get/set profile. Uses `RecordId::new()` for all record references
- [x] **2.2** `maelstrom-storage/surreal/devices.rs`: Implement `DeviceStore` — create device (replaces existing), get by user+device_id, get by access token, list devices, remove device, remove all. Uses `RecordId` for user links
- [x] **2.3** `maelstrom-api/extractors/auth.rs`: `AuthenticatedUser` extractor — validates `Authorization: Bearer` header or `access_token` query param, resolves user_id + device_id from storage
- [x] **2.4** `maelstrom-api/extractors/json.rs`: `MatrixJson<T>` extractor — returns `M_NOT_JSON` / `M_BAD_JSON` per spec
- [x] **2.5** `maelstrom-api/handlers/register.rs`: `POST /_matrix/client/v3/register` — username validation, argon2 password hashing, device creation, UIA with `m.login.dummy`, `inhibit_login` support
- [x] **2.6** `maelstrom-api/handlers/register.rs`: `GET /_matrix/client/v3/register/available` — username availability check with validation
- [x] **2.7** `maelstrom-api/handlers/auth.rs`: `GET /_matrix/client/v3/login` — returns `m.login.password` flow
- [x] **2.8** `maelstrom-api/handlers/auth.rs`: `POST /_matrix/client/v3/login` — password auth, supports `m.id.user` identifier + legacy `user` field, full/partial user_id
- [x] **2.9** `maelstrom-api/handlers/auth.rs`: `POST /_matrix/client/v3/logout` and `/logout/all` — revoke tokens, delete devices
- [x] **2.10** `maelstrom-api/handlers/account.rs`: `GET /_matrix/client/v3/account/whoami` — returns user_id, device_id, is_guest
- [x] **2.11** `maelstrom-api/handlers/account.rs`: `POST /_matrix/client/v3/account/deactivate` — account deactivation with UIA
- [x] **2.12** `maelstrom-api/handlers/account.rs`: `POST /_matrix/client/v3/account/password` — password change with UIA, optional `logout_devices`
- [x] **2.13** `maelstrom-api/handlers/profile.rs`: `GET/PUT /profile/{userId}/displayname`, `/avatar_url`, `GET /profile/{userId}` — with cross-user write protection
- [x] **2.14** `maelstrom-api/middleware/rate_limit.rs`: Sliding-window rate limiter on auth endpoints (login/register). 100 req/60s per IP. Returns `M_LIMIT_EXCEEDED` with `retry_after_ms`. Single node: in-memory. Cluster mode: NATS pub/sub (`maelstrom.ratelimit`) for cluster-wide enforcement — each instance broadcasts hits, all instances subscribe and merge counts.
- [x] **2.15** Write tests: register (8), auth (7), account (6), profile (6), storage mock (12) — 27 new tests, 57 total

#### Deliverable — Complete
Users can register, log in with password, manage devices, view/update profiles. 10 CS API endpoints implemented. Auth extractor validates Bearer tokens and query params. 57 total tests pass. Rate limiting deferred.

---

### Phase 3: Rooms & Membership
> Room creation, joining, leaving, inviting, banning, room directory

**Goal**: Users can create rooms, invite others, join/leave, and manage membership. Room state is stored as a graph in SurrealDB.

**Matrix Spec Refs**: [Room Creation](https://spec.matrix.org/latest/client-server-api/#creation), [Room Membership](https://spec.matrix.org/latest/client-server-api/#room-membership), [Room Directory](https://spec.matrix.org/latest/client-server-api/#listing-rooms)

#### Tasks

- [x] **3.1** `maelstrom-core/events/pdu.rs`: `StoredEvent` type with event_id, room_id, sender, type, state_key, content, origin_server_ts, unsigned, stream_position. Helpers: `generate_event_id()`, `generate_room_id()`, `timestamp_ms()`, `default_power_levels()`, `to_client_event()`
- [x] **3.2** Room event content types handled inline in handlers — m.room.create, m.room.member, m.room.power_levels, m.room.join_rules, m.room.history_visibility, m.room.name, m.room.topic
- [x] **3.3** Room version support: versions 1-11 validated in createRoom (rejects unknown with `M_UNSUPPORTED_ROOM_VERSION`), all versions listed as "stable" in `/capabilities` response
- [x] **3.4** `maelstrom-core/signatures/`: Event signing — completed in Phase 7 (Ed25519, canonical JSON, content/reference hashing)
- [x] **3.5** `maelstrom-storage/surreal/rooms.rs`: `RoomStore` — create room, get room, set/get membership, get joined rooms, get room members
- [x] **3.6** `maelstrom-storage/surreal/events.rs`: `EventStore` — store event, get event, paginated room timeline (forward/backward), events since token, room state map CRUD, stream position counter, txn_id dedup
- [x] **3.7** Room state via `room_state` table — maps (room_id, event_type, state_key) -> event_id, queried via `get_current_state()` and `get_state_event()`
- [x] **3.8** State resolution v2 — completed in Phase 7 (`maelstrom-core/state/mod.rs`): unconflicted extraction, power-level ordering, auth-chain resolution
- [x] **3.9** `POST /createRoom` — creates room with initial state events (create, member, power_levels, join_rules, history_visibility, optional name/topic), supports presets (private_chat/public_chat/trusted_private_chat), initial_state, invite list
- [x] **3.10** `POST /join/{roomIdOrAlias}` and `POST /rooms/{roomId}/join` — join rooms
- [x] **3.11** `POST /rooms/{roomId}/invite` — invite users with m.room.member event
- [x] **3.12** `POST /rooms/{roomId}/leave` — leave with membership check. Kick/ban/unban deferred
- [x] **3.13** `GET /joined_rooms` — lists user's joined room IDs
- [x] **3.14** `GET/PUT /rooms/{roomId}/state/{eventType}/{stateKey}` and `/state/{eventType}` and `GET /state` — full state endpoints in events handler
- [x] **3.15** Room directory — `PUT/GET/DELETE /directory/room/{alias}`, `GET /rooms/{roomId}/aliases`, `PUT /directory/list/room/{roomId}`, `GET/POST /publicRooms`
- [x] **3.16** Basic authorization: membership checks before room operations
- [x] **3.17** Write tests: rooms (6), events (6) — room creation, name/topic, leave, invite+join, send, dedup, get event, messages, state get/set

#### Deliverable — Complete
Room lifecycle works: create, join, leave, invite. State events stored and queryable. 26 CS API endpoints total. Kick/ban/unban, room directory, state resolution deferred to later phases.

**Deferred items** (not needed for client connectivity):
- Room aliases and public room directory
- Kick, ban, unban membership operations
- Power level enforcement beyond basic membership checks
- State resolution algorithm v2
- Room version definitions

---

### Phase 4: Messaging & Sync
> Sending/receiving messages, sync endpoint, typing, receipts, presence

**Goal**: Users can send messages, receive them via `/sync`, and see typing indicators and read receipts. This is where the homeserver becomes usable with real clients.

**Matrix Spec Refs**: [Sending Events](https://spec.matrix.org/latest/client-server-api/#sending-events-to-a-room), [Syncing](https://spec.matrix.org/latest/client-server-api/#syncing), [Sliding Sync](https://spec.matrix.org/latest/client-server-api/#sliding-sync)

#### Tasks

- [x] **4.1** `PUT /rooms/{roomId}/send/{eventType}/{txnId}` — send message events with txn_id deduplication
- [x] **4.2** `GET /rooms/{roomId}/event/{eventId}` — get single event
- [x] **4.3** `GET /rooms/{roomId}/messages` — paginated message history (forward/backward via `dir`, `from`, `limit`)
- [x] **4.4** `PUT /rooms/{roomId}/redact/{eventId}/{txnId}` — redact events with reason, txn_id dedup
- [x] **4.5** Stream position counter in SurrealDB (`stream_counter:global`), monotonically incremented per event, used as sync tokens
- [x] **4.6** `GET /sync` — initial sync + incremental sync + **long-polling** with Notifier integration (`tokio::select!` between notification and timeout)
- [x] **4.7** Sliding Sync — `POST /sync` handler with room lists, ranges, required_state, timeline, extensions (to_device, typing, receipts). 350 lines in sync.rs.
- [x] **4.8** `PUT /rooms/{roomId}/typing/{userId}` — typing notifications with expiry, stored in SurrealDB, delivered via sync ephemeral events
- [x] **4.9** `POST /rooms/{roomId}/receipt/{receiptType}/{eventId}` — read receipts stored in SurrealDB, delivered via sync ephemeral events
- [x] **4.10** `GET/PUT /presence/{userId}/status` — presence (online/offline/unavailable) with last_active_ago calculation
- [x] **4.11** Transaction ID idempotency via `txn_id` table — dedup across device+txn_id
- [x] **4.12** `POST /search` — full-text search using SurrealDB BM25 indexing on `content.body` with Snowball English stemming, relevance-ranked results, room filtering
- [x] **4.13** Write tests: sync (4), rooms (6), events (6) — 73 total tests passing
- [x] **4.14** Notifier system: `Notifier` trait with `LocalNotifier` (single-node, `tokio::broadcast`) and `NatsNotifier` (cluster, NATS pub/sub). All event/typing/receipt/presence handlers publish notifications. Sync handler subscribes and long-polls.
- [x] **4.15** Cluster configuration: `[cluster]` section in config with `mode` (single/cluster) and `nats_url`. Docker Compose updated with NATS service.
- [x] **4.16** Element Web stub endpoints: `/capabilities`, `/filter`, `/account_data`, `/pushrules`, `/pushers`, `/voip/turnServer`, `/devices`, `/keys/upload`, `/keys/query`, `/keys/device_signing/upload`, `/keys/signatures/upload`, `/keys/claim`, `/keys/changes`, `/sendToDevice`, `/rooms/{roomId}/members`
- [x] **4.17** Code review fixes: removed all `unwrap()` from production code, `spawn_blocking` for Argon2, `store_state_event` helper to deduplicate room creation, `storage_error()` conversion with proper error discrimination, transaction-based upserts, N+1 query fix in `get_current_state`, safe NATS encoding, `HashSet` for sync membership checks, proper `Datetime` conversion from SurrealDB

- [x] **4.18** Sliding Sync (`POST /sync`): room list sorted by recency, range-based windowing with SYNC ops, per-room required_state + timeline, typing/receipt extensions. Element X compatible.

#### Deliverable — Complete
Element Web and Element X can connect. Real-time sync (traditional + sliding), typing, receipts, presence, full-text search. 40+ CS API endpoints, 73 tests passing. Single-node and cluster mode.

---

### Phase 5: End-to-End Encryption (E2EE)
> Key upload, key query, key claim, to-device messages

**Goal**: Clients can perform key exchange and send encrypted messages.

**Matrix Spec Refs**: [E2EE](https://spec.matrix.org/latest/client-server-api/#end-to-end-encryption)

#### Tasks

- [x] **5.1** `KeyStore` trait + `ToDeviceStore` trait with SurrealDB implementation (`surreal/keys.rs`) and mock. Schema: `device_key`, `one_time_key`, `cross_signing_key`, `key_signature`, `to_device_message` tables.
- [x] **5.2** `POST /keys/upload` — stores device keys + OTKs, returns OTK counts by algorithm
- [x] **5.3** `POST /keys/query` — queries device keys + cross-signing keys for requested users
- [x] **5.4** `POST /keys/claim` — claims OTKs (consumed on claim, deleted from storage)
- [x] **5.5** `GET /keys/changes` — stub (returns empty changed/left)
- [x] **5.6** `POST /keys/device_signing/upload` — stores cross-signing keys. `POST /keys/signatures/upload` — stub accepting signatures.
- [x] **5.7** `PUT /sendToDevice/{eventType}/{txnId}` — stores to-device messages per target user+device, handles `"*"` wildcard
- [x] **5.8** To-device messages delivered via sliding sync extensions (to_device extension)
- [x] **5.9** Tests: `tests/keys_test.rs` — 5 tests (upload device keys, upload OTKs, query keys, claim OTKs, cross-signing upload)

#### Deliverable — Complete
E2EE key management operational. Device keys, OTKs, cross-signing keys stored and queryable. To-device messaging works. Element can complete key setup. Tests pending.

---

### Phase 6: Media
> Upload, download, thumbnails, URL previews, retention policy

**Goal**: Users can upload/download files, images render with thumbnails, and admins can configure retention policies.

**Matrix Spec Refs**: [Content Repository](https://spec.matrix.org/latest/client-server-api/#content-repository)

#### Tasks

- [x] **6.1** `maelstrom-media/client.rs`: S3 client wrapper using `aws-sdk-s3` — connect to RustFS, upload, download, delete, exists
- [x] **6.2** Upload handling in `handlers/media.rs` — content-type validation, size limits, generate MXC URI, store blob in RustFS + metadata in SurrealDB
- [x] **6.3** Download handling in `handlers/media.rs` — resolve MXC URI to S3 key, return response with content headers
- [x] **6.4** `maelstrom-media/thumbnail.rs`: Real thumbnail generation using `image` crate — scale and crop resize methods, PNG output, falls back to original for non-images
- [x] **6.5** `maelstrom-api/handlers/media.rs`: Both legacy `/_matrix/media/v3/*` and new `/_matrix/client/v1/media/*` endpoints — upload, download, download with filename, thumbnail, config, preview_url
- [x] **6.6** `maelstrom-media/preview.rs`: `GET preview_url` — real OpenGraph metadata fetching via `reqwest` + `scraper`, with fallbacks to `<title>` and `<meta name="description">`
- [x] **6.7** `maelstrom-api/handlers/media.rs`: `GET config` — returns `m.upload.size`
- [x] **6.8** `maelstrom-media/retention.rs`: Retention policy background task — configurable `max_age_days`, `sweep_interval_secs`, batch deletion from S3 + DB. Config in `[media]` section.
- [x] **6.9** `maelstrom-storage/surreal/media.rs`: Media metadata storage — MXC URI mapping, upload timestamps, file sizes, content types, quarantine status
- [x] **6.10** Write tests: `tests/media_test.rs` + `maelstrom-media` unit tests — 16 total (8 integration, 4 thumbnail, 4 OG preview)

#### Deliverable — Complete
All Phase 6 features fully implemented. File uploads/downloads work. Thumbnails generated with real image resizing (scale/crop). URL previews fetch OpenGraph metadata. Retention policy runs as background task. Both v3 and v1 media endpoints supported.

---

### Phase 7: Federation
> Server-to-server API, event signing, key management, remote joins, transaction processing

**Goal**: Maelstrom can federate with other Matrix homeservers (Synapse, Dendrite, Conduwuit). Users on different servers can communicate.

**Matrix Spec Refs**: [Server-Server API](https://spec.matrix.org/latest/server-server-api/)

#### Tasks

- [x] **7.1** `maelstrom-core/signatures/keys.rs`: Ed25519 keypair generation, storage/loading, signing and verification. Unpadded base64 encoding.
- [x] **7.2** `maelstrom-federation/signing.rs`: HTTP request signing — `X-Matrix` auth scheme with sign/verify/parse. Canonical JSON serialization.
- [x] **7.3** `maelstrom-federation/key_server.rs`: `GET /_matrix/key/v2/server` — serve own self-signed signing keys with `verify_keys` and `valid_until_ts`
- [x] **7.4** `maelstrom-federation/client.rs`: Outbound federation HTTP client — `.well-known/matrix/server` discovery, port 8448 fallback, endpoint caching, signed GET/PUT
- [x] **7.5** `maelstrom-federation/receiver.rs`: `PUT /_matrix/federation/v1/send/{txnId}` — receive transactions, process PDUs, store events, transaction deduplication
- [x] **7.6** PDU processing — parse federation PDUs into StoredEvent, store with federation fields, update room state for state events (full signature verification deferred to hardening)
- [x] **7.7** `maelstrom-federation/joins.rs`: `make_join` + `send_join` (v2) — event templates with auth_events/prev_events, store join, return room state + auth chain
- [x] **7.8** `maelstrom-federation/joins.rs`: `make_leave` + `send_leave` (v2) — remote leave protocol
- [x] **7.9** `maelstrom-federation/backfill.rs`: `GET /backfill/{roomId}` + `POST /get_missing_events/{roomId}` — historical event retrieval
- [x] **7.10** `maelstrom-federation/state.rs`: `GET /state/{roomId}`, `GET /state_ids/{roomId}`, `GET /event/{eventId}` — room state and event queries
- [x] **7.11** `maelstrom-federation/sender.rs`: `TransactionSender` — per-destination queue, batch up to 50 PDUs, exponential backoff retry (1s to 1hr)
- [x] **7.12** Federation EDU handling — inbound typing, presence, receipts, device list updates processed and stored. Outbound EDU queuing in TransactionSender.
- [x] **7.13** Media over federation — remote MXC URI proxying. Downloads for remote `server_name` fetched from origin server via HTTPS.
- [x] **7.14** E2EE over federation — `POST /_matrix/federation/v1/user/keys/query` serves device keys + cross-signing keys for local users.
- [x] **7.15** Tests: `tests/federation_test.rs` (7 tests) + 15 unit tests in `maelstrom-core` (crypto) + 3 in `maelstrom-federation` (signing)

#### Deliverable — Complete
Full federation layer implemented. Ed25519 signing, key server + notary, inbound/outbound transactions, remote join/leave, backfill, state queries, EDU propagation (typing/presence/receipts/device lists), media federation proxy, cross-server E2EE key queries, SRV DNS discovery, state resolution v2. Server key auto-generated on startup and persisted. Federation router merged into main app.

---

### Phase 8: Matrix 2.0+ Features
> Threads, relations, reactions, polls, spaces, knocking, restricted rooms, account suspension

**Goal**: Full Matrix 2.0+ feature coverage for modern client compatibility.

**Matrix Spec Refs**: [Relations](https://spec.matrix.org/latest/client-server-api/#forming-relationships-between-events), [Threads](https://spec.matrix.org/latest/client-server-api/#threading), [Spaces](https://spec.matrix.org/latest/client-server-api/#spaces), [Knocking](https://spec.matrix.org/latest/client-server-api/#knocking-on-rooms)

#### Tasks

- [x] **8.1** `maelstrom-api/handlers/relations.rs`: `GET /rooms/{roomId}/relations/{eventId}` with filtering by rel_type and event_type, pagination
- [x] **8.2** Reaction aggregation: `build_aggregations()` bundles reaction counts (m.annotation) into unsigned.m.relations
- [x] **8.3** Event editing: `m.replace` relation storage and `get_latest_edit()` for serving edited content
- [x] **8.4** `maelstrom-api/handlers/threads.rs`: `GET /rooms/{roomId}/threads` — thread root listing with aggregation summaries
- [x] **8.5** Thread-aware relations: `m.thread` relation stored via `extract_and_store_relation()` on event send, thread summaries in aggregations
- [x] **8.6** Spaces: `GET /rooms/{roomId}/hierarchy` — BFS traversal of `m.space.child` state events, configurable depth/limit/suggested_only
- [x] **8.7** Knocking: `POST /knock/{roomIdOrAlias}` — validates join_rule is "knock" or "knock_restricted", creates m.room.member with membership "knock"
- [x] **8.8** Restricted rooms: `m.room.join_rules` with `restricted` type supported via state events (enforcement in join handler to be hardened)
- [x] **8.9** Polls: `m.poll.start`, `m.poll.response`, `m.poll.end` — handled as standard events with relation tracking via m.relates_to
- [x] **8.10** Account locking/suspension: `is_deactivated` field on user records, checked in auth extractor (full MSC3939 locked/suspended states can be added as fields)
- [x] **8.11** Reporting: `POST /rooms/{roomId}/report/{eventId}` with reason and score, stored in `event_report` table
- [x] **8.12** Policy servers: `m.policy.rule.*` events stored as standard state events (server-side enforcement to be hardened in Phase 10)
- [x] **8.13** Tests: `tests/phase8_test.rs` — 7 tests (reactions, threads, knocking, reporting, spaces, relation storage)

#### Deliverable — Complete
Matrix 2.0+ features implemented. Relations (reactions, edits, threads) with storage and aggregation. Space hierarchy traversal. Room knocking. Event reporting. Relation extraction on event send. All endpoints wired into router.

---

### Phase 9: Admin API & Operations
> Admin dashboard backend, moderation tools, metrics, media retention, Synapse migration

**Goal**: Production-ready operations tooling.

#### Tasks

- [x] **9.1** `maelstrom-admin/handlers/users.rs`: Get user details (profile, devices, rooms), deactivate/reactivate, reset password (Argon2), admin grant/revoke, list devices
- [x] **9.2** `maelstrom-admin/handlers/rooms.rs`: List rooms, room details (members, state, metadata), shutdown room (kick all members)
- [x] **9.3** `maelstrom-admin/handlers/media.rs`: List user media, quarantine/unquarantine media by server_name/media_id
- [x] **9.4** `maelstrom-admin/handlers/federation.rs`: Federation stats (signing keys, server identity)
- [x] **9.5** `maelstrom-admin/handlers/server.rs`: Server info (version, uptime, memory, CPU, DB health), detailed health check
- [x] **9.6** `maelstrom-admin/handlers/reports.rs`: List abuse reports endpoint
- [x] **9.7** Prometheus metrics: `/_maelstrom/admin/v1/metrics` — uptime, memory, DB connectivity in Prometheus text format
- [x] **9.8** Health checks: `/_health/live` and `/_health/ready` (from Phase 1, still operational)
- [x] **9.9** Structured logging: `tracing` + `tracing-subscriber` with env-filter, request tracing via tower-http
- [x] **9.10** Admin auth: `AdminUser` extractor checks Bearer token + `is_admin` flag on user account. Non-admin users get 403.
- [x] **9.11** Tests: `tests/admin_test.rs` — 6 tests (auth required, non-admin rejected, server info, metrics, get user, dashboard HTML)

**Admin Dashboard (SSR + Datastar):**
- Askama templates with semantic HTML (HTML Purist compliant — no inline styles, semantic class names, `<dl>`, `<nav>`, `<article>`, `<section>`)
- CSS custom properties for theming, dark/light mode via `prefers-color-scheme`, `prefers-reduced-motion` support
- Datastar loaded via CDN for progressive enhancement
- Pages: Dashboard (server overview), Users, Rooms, Federation
- Static CSS served via `tower-http::services::ServeDir`

#### Deliverable — Complete
Full admin API (JSON) + admin dashboard (SSR HTML). Admin auth via is_admin flag. Prometheus metrics. 6 tests passing.

---

### Phase 10: Complement Testing & Hardening
> Full Complement pass, client compatibility testing, performance tuning

**Goal**: 100% Complement pass rate for CS API + Federation. Validated with real clients. Performance meets targets.

**Current**: 336/538 (62.5%) → **Target**: 500+/538 (93%+)  
**Federation**: Infrastructure complete, hardening in progress. Signature verification implemented.

#### 10A — CS API Hardening (complete)

- [x] **10.1** Complement-compatible Dockerfile + CI pipeline
- [x] **10.2** Media made optional; first-user-is-admin bootstrap
- [x] **10.3** Typing: always include m.typing in sync ephemeral (even empty user_ids for stop)
- [x] **10.4** Account data: DELETE endpoints (MSC3391), PUT-empty-deletes, device cleanup on delete
- [x] **10.5** Keys: UIA on device_signing/upload (MSC3967), key claim ORDER BY, backup replacement rules
- [x] **10.6** Relations: pagination (DESC order, next_batch cursor), FilterMessagesByRelType on /messages
- [x] **10.7** Members: /members?at= support, unsigned.membership on all client events
- [x] **10.8** User directory: exact match for mxid searches
- [x] **10.9** Rooms: unban-via-invite, is_direct on invite content, owned state_key validation
- [x] **10.10** Sync: timeline gap limited flag, lazy-load members in initial sync, leave event visibility
- [x] **10.11** Push rules: predefined server-default rules (poll, reaction, suppress_notices, etc.)
- [x] **10.12** Threads: ordering by latest reply (GROUP BY + math::max), thread_id on receipts (MSC4102)
- [x] **10.13** Push rule carry-over on room upgrade
- [x] **10.14** Type system: `matrix/` module with `Pdu`, `ClientEvent`, `StrippedEvent`, typed `Content` enum, `CanonicalJson`, `RoomVersion`, `Membership`, `JoinRule`, `HistoryVisibility`, `PowerLevelContent`, `event_type` constants, typed EDUs
- [x] **10.15** Migrate all handlers to `matrix::*` types — replaced all raw string comparisons with `Membership`, `JoinRule`, `HistoryVisibility` enums and `event_type::*` constants (~80 call sites)
- [x] **10.16** Consolidate `maelstrom-core` — moved `error.rs`, `json.rs`, `signatures/`, `state/`, `ephemeral.rs` into `matrix/` module. `lib.rs` is now `pub mod matrix;`
- [x] **10.17** Code quality — replaced hand-rolled percent encoding with `urlencoding` crate, replaced `std::sync::Mutex` with `DashMap` in async code (notify, federation sender/client), extracted `require_membership` helper, DRYed signing.rs, fixed `generate_event_id` to use base64 crate, made 18+ methods `const fn`, added rustdoc to ~65 files
- [x] **10.18** Receipt schema fix: added `thread_id` field to SCHEMAFULL receipt table, normalized to `String` (empty = unthreaded)
- [x] **10.19** Sync race condition: subscribe to notifier BEFORE checking for events (prevents missed receipts/typing)
- [x] **10.20** Broadcast channel fix: handle `Lagged` error in notification forwarding tasks (was silently killing subscribers)
- [x] **10.21** Account data deletion (MSC3391): sentinel-based approach — deleted entries appear in sync with empty `{}` content, GET returns 404
- [x] **10.22** Poll push rules: added all stable + unstable poll push rules, fixed `actions: []` vs `["dont_notify"]`
- [x] **10.23** Device notification settings: sync notification after MSC3890 cleanup on device deletion
- [x] **10.24** Owned state keys: power level check uses `users_default` (not `state_default`) when state_key matches sender
- [x] **10.25** `server_name` query param: changed from `Option<Vec<String>>` to `Option<String>` — Complement sends single value, not array
- [x] **10.26** Detailed rustdoc: comprehensive documentation across all 95 source files with Matrix protocol explanations
- [ ] **10.27** Search: back-pagination (next_batch offset), context around results (partially implemented)
- [ ] **10.28** Remaining CS API edge cases: redaction of unknown events in sync, TxnId with refresh tokens, typing stop detection

#### 10B — Federation Hardening ← ACTIVE SPRINT

**Current state**: 336/538 Complement tests passing (62.5%). Federation crate is ~2000 LOC across 13 files. Signature verification, server ACL enforcement, device list EDU propagation, and profile queries all implemented. Remaining failures:

| Category | Fail | Status |
|----------|------|--------|
| PartialStateJoin | 59 | Defer — MSC3706, complex protocol |
| Sync edge cases | 51 | Active — state resolution, event ordering, lazy loading |
| State (MSCs) | 40 | Mixed — DelayedEvents (MSC4140), MSC4222 deferred; owned state fixed |
| Members | 19 | Active — device list tracking, federation join edge cases |
| Messages | 19 | Active — outbound backfill needed |
| InviteFiltering | 11 | MSC4155 (per-user invite permissions), not server_acl |
| Relations | 11 | Active — aggregation and pagination gaps |
| DeviceListUpdates | 6 | Partially fixed — join detection done, leave/remote tracking remaining |
| ThreadSubscriptions | 7 | Defer — MSC4306, unstable |
| ServerNotices | 8 | Defer — not a standard feature |
| AsyncUpload | 6 | Defer — not implemented |

**Step 1: Foundation — make federation actually work in Complement** ✅
- [x] **10B.1** Complement federation Docker setup: TLS certificates, `.well-known/matrix/server`, two-server communication
- [x] **10B.2** `POST /join/{roomIdOrAlias}` for remote rooms: CS API initiates make_join → sign → send_join
- [x] **10B.3** Key server responses: `GET /_matrix/key/v2/server` returns properly signed JSON
- [x] **10B.4** Transaction receiving: `PUT /_matrix/federation/v1/send/{txnId}` processes PDUs

**Step 2: Invite over federation** (partially complete)
- [x] **10B.5** `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}` — accept invites from remote servers
- [x] **10B.6** `PUT /_matrix/federation/v1/invite/{roomId}/{eventId}` — v1 compat
- [x] **10B.7** Outbound invite: CS API invite for remote users routes through federation
- [x] **10B.8** `m.room.server_acl` checking on inbound federation (receiver, joins, invites)

**Step 3: Profile over federation** ✅
- [x] **10B.9** `GET /_matrix/federation/v1/query/profile` — serve local user profiles with proper validation
- [x] **10B.10** Outbound profile query: CS API profile handlers query remote servers via federation

**Step 4: Device list updates over federation** (partially complete)
- [x] **10B.11** On device change (key upload, device delete), queue `m.device_list_update` EDU to remote servers
- [x] **10B.12** On inbound `m.device_list_update` EDU, record change position for sync `device_lists.changed`
- [ ] **10B.13** Track which servers need device list updates via per-server stream tracking
- [x] **10B.14** `device_lists.changed` in sync includes users from newly joined rooms

**Step 5: Signature verification** (partially complete)
- [x] **10B.15** Verify signatures on inbound PDUs: `resolve_server_key` fetches+caches remote keys, `verify_event_signature` checks Ed25519. Present-but-invalid sigs hard-rejected, missing sigs warned.
- [ ] **10B.16** Verify signatures on inbound `send_join`/`send_leave` responses
- [ ] **10B.17** Verify signatures on fetched remote server keys (key_server notary responses)
- [ ] **10B.18** Reject events with invalid or missing signatures (`TestInboundFederationRejectsEventsWithRejectedAuthEvents`)

**Step 6: Event authorization** (spec compliance)
- [ ] **10B.19** Implement full event auth rules per room version: check power levels, membership, join_rules, etc. on inbound PDUs
- [ ] **10B.20** Auth chain calculation: return minimal auth chain for state responses (not entire state)
- [ ] **10B.21** Auth event selection: correctly select auth events when creating events (m.room.create, m.room.join_rules, m.room.power_levels, sender's m.room.member)

**Step 7: Backfill and messages over federation** (~3 tests)
- [ ] **10B.22** Outbound backfill: when `/messages` hits the beginning of local history, request earlier events from the room's origin server via `GET /_matrix/federation/v1/backfill/{roomId}`
- [ ] **10B.23** Fix `get_missing_events` to properly walk the DAG between earliest_events and latest_events
- [ ] **10B.24** Store backfilled events with correct stream_position ordering

**Step 8: Remaining federation features** (~5 tests)
- [ ] **10B.25** Federation redactions: `PUT /send/{txnId}` with redaction PDUs must be processed correctly
- [ ] **10B.26** Remote room alias queries: `GET /_matrix/federation/v1/query/directory` — resolve aliases on remote servers
- [ ] **10B.27** Presence over federation: properly relay presence EDUs between servers
- [ ] **10B.28** is_direct flag over federation: preserve in federated invite events

**Step 9: Partial state join** (56 tests — complex, phased approach)
> MSC3706: "Faster joins". Server joins a room and starts participating immediately with partial state, then backfills the full state in the background. This is the single largest test category.

- [ ] **10B.29** Phase A — Accept partial state joins: respond to `send_join` with `org.matrix.msc3706.partial_state: true` support
- [ ] **10B.30** Phase B — Initiate partial state joins: on outbound join, request partial state and begin participating before full state arrives
- [ ] **10B.31** Phase C — Background state resync: after partial join, fetch full state via `/state_ids` + `/state` and resolve
- [ ] **10B.32** Phase D — Sync during partial state: serve sync responses during resync, block `/members` until complete
- [ ] **10B.33** Phase E — Edge cases: device list tracking during partial state, leave during resync, event auth with partial state

**Step 10: DNS and transport hardening**
- [ ] **10B.34** Replace `dig` shell exec for SRV lookups with `hickory-resolver` (pure Rust DNS)
- [ ] **10B.35** Configurable TLS validation: accept custom CA certs, option to require valid certs in production
- [ ] **10B.36** Federation rate limiting: per-origin request limits to prevent flooding
- [ ] **10B.37** Transaction deduplication cleanup: TTL-based expiry of old txn_id records

#### 10C — Client Compatibility & Production Readiness

- [ ] **10.16** Client compatibility testing: Element Web, Element X, FluffyChat, nheko
- [ ] **10.17** Performance benchmarking: large rooms, message throughput, sync latency
- [ ] **10.18** Horizontal scaling validation: 3+ instances, consistency checks
- [ ] **10.19** Chaos testing: kill nodes, verify recovery
- [ ] **10.20** Security audit: input validation, auth, rate limiting, OWASP top 10
- [ ] **10.21** Documentation: deployment guide, config reference, architecture overview

#### Deliverable
Production-ready release candidate. 93%+ Complement (CS API + Federation). Validated with clients. Performance targets met.

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
| 1 | Foundation | **Complete** | — |
| 2 | Authentication & Registration | **Complete** | Phase 1 |
| 3 | Rooms & Membership | **Complete** | Phase 2 |
| 4 | Messaging & Sync | **Complete** (incl. sliding sync) | Phase 3 |
| 5 | E2EE | **Complete** | Phase 4 |
| 6 | Media | **Complete** | Phase 1 |
| 7 | Federation | **Complete** (infrastructure) | Phase 4 |
| 8 | Matrix 2.0+ Features | **Complete** | Phase 4 |
| 9 | Admin & Operations | **Complete** | Phase 6, 7 |
| 10A | CS API Hardening | **Complete** — types, code quality, rustdoc, receipts, presence, typing, push rules, account data | Phase 7, 8 |
| 10B | Federation Hardening | **Active** — Steps 1-4 done, Step 5 (signatures) in progress, Steps 6-10 remaining | Phase 7 |
| 10C | Client Compat & Prod | Not Started | Phase 10A, 10B |
| 11 | Migration & 1.0 | Not Started | Phase 10 |

### Current Focus: Phase 10B — Federation Hardening

**Objective**: Complete Matrix federation spec compliance. Pass Complement federation tests.

**Stats**: 95 source files, ~22K LOC, 107 unit tests, 336/538 Complement tests (62.5%), zero clippy warnings.

**Next up**: Step 5 (signature verification on joins), Step 6 (event authorization rules), Step 7 (outbound backfill).

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
