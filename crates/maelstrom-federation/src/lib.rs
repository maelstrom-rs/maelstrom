//! # Matrix Federation (Server-to-Server API)
//!
//! This crate implements the **Matrix Server-Server API**, commonly called "federation."
//! Federation is how Matrix homeservers talk to each other -- when a user on `alice.com`
//! sends a message to a room that has members on `bob.org`, the `alice.com` server
//! delivers that message to `bob.org` over federation.
//!
//! ## The Transaction Model
//!
//! Federation communication is built around **transactions**. A transaction is a batch
//! of events sent from one server to another via
//! `PUT /_matrix/federation/v1/send/{txnId}`. Each transaction contains:
//!
//! - **PDUs** (Persistent Data Units) -- room events like messages, state changes, and
//!   membership updates. These are stored permanently and form the room's event DAG
//!   (Directed Acyclic Graph).
//! - **EDUs** (Ephemeral Data Units) -- transient data like typing notifications, read
//!   receipts, and presence updates. These are not persisted by the receiving server.
//!
//! ## Crate Organization
//!
//! | Module          | Purpose                                               |
//! |-----------------|-------------------------------------------------------|
//! | [`client`]      | Outbound HTTP client with server discovery and signing |
//! | [`signing`]     | X-Matrix request signing and verification              |
//! | [`key_server`]  | Publishing and fetching server signing keys            |
//! | [`sender`]      | Outbound transaction queuing with batching and retry   |
//! | [`receiver`]    | Inbound transaction processing (PDUs and EDUs)         |
//! | [`joins`]       | Federation join/leave protocol (make/send handshake)   |
//! | [`invite`]      | Federation invite flow for remote users                |
//! | [`backfill`]    | Historical event retrieval and DAG gap filling         |
//! | [`state`]       | Room state and individual event queries                |
//! | [`queries`]     | Profile and room alias lookups for remote servers      |
//! | [`user_keys`]   | Cross-server device key queries for E2EE               |
//! | [`router`]      | Axum router assembling all federation endpoints        |
//!
//! ## Shared State
//!
//! All federation endpoints share a [`FederationState`] instance, which provides
//! access to storage, the server's signing key, ephemeral data, and the outbound
//! federation HTTP client. It is cheaply cloneable (wraps an `Arc`).

pub mod backfill;
pub mod client;
pub mod invite;
pub mod joins;
pub mod key_server;
pub mod queries;
pub mod receiver;
pub mod router;
pub mod sender;
pub mod signing;
pub mod state;
pub mod user_keys;

use std::sync::Arc;

use maelstrom_core::matrix::ephemeral::EphemeralStore;
use maelstrom_core::matrix::id::ServerName;
use maelstrom_core::matrix::keys::KeyPair;
use maelstrom_storage::traits::Storage;

/// Shared state for all federation endpoints.
///
/// This is the central context object passed to every Axum handler in the federation
/// router. It bundles together everything a federation endpoint needs:
///
/// - **Storage** -- persistent database access (events, rooms, memberships, keys)
/// - **EphemeralStore** -- in-memory store for typing, presence, and other transient data
/// - **KeyPair** -- this server's Ed25519 signing key, used to sign outbound requests
///   and events
/// - **ServerName** -- this server's canonical name (e.g., `matrix.example.com`)
/// - **FederationClient** -- HTTP client for making outbound federation requests
///
/// `FederationState` is cheaply cloneable via an inner `Arc`, so it can be shared
/// across all Axum handlers without additional wrapping.
#[derive(Clone)]
pub struct FederationState {
    inner: Arc<FederationStateInner>,
}

/// Inner storage for [`FederationState`], held behind an `Arc` for cheap cloning.
struct FederationStateInner {
    storage: Box<dyn Storage>,
    ephemeral: Arc<EphemeralStore>,
    signing_key: KeyPair,
    server_name: ServerName,
    federation_client: client::FederationClient,
}

impl FederationState {
    /// Create a new federation state with the given storage backend, ephemeral store,
    /// signing key, and server name. This also constructs the outbound
    /// [`FederationClient`](client::FederationClient) using the same signing key.
    pub fn new(
        storage: impl Storage,
        ephemeral: Arc<EphemeralStore>,
        signing_key: KeyPair,
        server_name: ServerName,
    ) -> Self {
        let fed_client = client::FederationClient::new(signing_key.clone(), server_name.clone());

        Self {
            inner: Arc::new(FederationStateInner {
                storage: Box::new(storage),
                ephemeral,
                signing_key,
                server_name,
                federation_client: fed_client,
            }),
        }
    }

    /// Access the persistent storage backend.
    pub fn storage(&self) -> &dyn Storage {
        &*self.inner.storage
    }

    /// Access the in-memory ephemeral data store (typing, presence, etc.).
    pub fn ephemeral(&self) -> &EphemeralStore {
        &self.inner.ephemeral
    }

    /// Access this server's Ed25519 signing key pair.
    pub fn signing_key(&self) -> &KeyPair {
        &self.inner.signing_key
    }

    /// This server's canonical name (e.g., `matrix.example.com`).
    pub fn server_name(&self) -> &ServerName {
        &self.inner.server_name
    }

    /// Access the outbound federation HTTP client for making requests to other servers.
    pub fn client(&self) -> &client::FederationClient {
        &self.inner.federation_client
    }
}
