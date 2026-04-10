//! # maelstrom-api -- Matrix Client-Server API
//!
//! This crate is the HTTP layer that Matrix clients (Element, FluffyChat, etc.) talk to.
//! It implements the [Matrix Client-Server API](https://spec.matrix.org/latest/client-server-api/)
//! on top of [Axum](https://docs.rs/axum), Rust's async web framework.
//!
//! ## Crate layout
//!
//! | Module | Purpose |
//! |---|---|
//! | [`router`] | Assembles every route into a single `axum::Router`. Start here to see the full API surface. |
//! | [`handlers`] | One sub-module per spec section (rooms, sync, keys, ...). Each file exposes a `routes()` function and the async handler functions that implement the endpoints. |
//! | [`extractors`] | Axum extractors that act as middleware -- [`extractors::AuthenticatedUser`] gates authentication, [`extractors::MatrixJson`] enforces Matrix-compliant JSON parsing. |
//! | [`state`] | [`state::AppState`] -- the shared context (storage, notifier, federation client, server name, etc.) passed to every handler via Axum's `State` extractor. |
//! | [`notify`] | Pub/sub notification system that connects event-producing handlers (send message, set typing, ...) to the `/sync` long-poll loop. |
//! | [`middleware`] | Tower middleware layers (compression, CORS, tracing). |
//! | [`gossip`] | Cluster gossip for multi-node deployments. |
//!
//! ## Request / response pattern
//!
//! Every handler has a signature like:
//!
//! ```rust,ignore
//! async fn my_handler(
//!     State(state): State<AppState>,       // shared state
//!     user: AuthenticatedUser,             // auth gate (optional -- omit for public endpoints)
//!     MatrixJson(body): MatrixJson<Req>,   // typed JSON body
//! ) -> Result<Json<Resp>, MatrixError>
//! ```
//!
//! `MatrixError` implements `IntoResponse`, so Axum automatically serializes it to
//! the JSON error format the Matrix spec requires (`errcode` + `error` fields).
//! Successful responses are serialized by `axum::Json`.
//!
//! ## Adding a new endpoint
//!
//! 1. Create (or extend) a handler file in `handlers/` with your async function.
//! 2. Add a `routes()` function that returns a `Router<AppState>` with the path and method.
//! 3. Merge it into `router::build()`.
//! 4. Done -- Axum handles deserialization, serialization, and error mapping for you.

pub mod extractors;
pub mod gossip;
pub mod handlers;
pub mod middleware;
pub mod notify;
pub mod router;
pub mod state;
