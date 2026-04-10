//! SurrealDB storage implementation.
//!
//! [SurrealDB](https://surrealdb.com) is a multi-model database that supports
//! documents, graphs, full-text search, and time-series data in a single engine.
//! Maelstrom uses it as the sole production backend.
//!
//! # Connection
//!
//! [`connection`] handles connecting to SurrealDB (WebSocket, in-memory, or
//! RocksDB), authenticating, selecting the namespace/database, and running the
//! schema bootstrap.
//!
//! # Schema
//!
//! [`schema`] embeds the SurrealQL schema file (`db/schema.surql`) at compile
//! time via `include_str!` and executes it on every startup.  The schema uses
//! `IF NOT EXISTS` throughout, making it idempotent and safe for rolling
//! deployments.
//!
//! # Sub-modules
//!
//! Each sub-module implements one or more storage sub-traits:
//!
//! | Module          | Trait(s) implemented                       |
//! |-----------------|--------------------------------------------|
//! | [`users`]       | [`UserStore`](crate::traits::UserStore)    |
//! | [`devices`]     | [`DeviceStore`](crate::traits::DeviceStore)|
//! | [`rooms`]       | [`RoomStore`](crate::traits::RoomStore)    |
//! | [`events`]      | [`EventStore`](crate::traits::EventStore)  |
//! | [`receipts`]    | [`ReceiptStore`](crate::traits::ReceiptStore) |
//! | [`keys`]        | [`KeyStore`](crate::traits::KeyStore) + [`ToDeviceStore`](crate::traits::ToDeviceStore) |
//! | [`account_data`]| [`AccountDataStore`](crate::traits::AccountDataStore) |
//! | [`media`]       | [`MediaStore`](crate::traits::MediaStore)  |
//! | [`federation`]  | [`FederationKeyStore`](crate::traits::FederationKeyStore) |
//! | [`relations`]   | [`RelationStore`](crate::traits::RelationStore) |

mod account_data;
pub mod connection;
mod devices;
mod events;
mod federation;
mod keys;
mod media;
mod receipts;
mod relations;
mod rooms;
pub mod schema;
mod users;

use async_trait::async_trait;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;

use crate::traits::*;

/// SurrealDB-backed storage implementation.
///
/// This is the primary (and only production) storage backend for Maelstrom.
/// It wraps a `Surreal<Any>` client handle which is cheaply cloneable --
/// internally it holds an `Arc` to the connection pool, so cloning
/// `SurrealStorage` is the same as cloning the client.
#[derive(Clone)]
pub struct SurrealStorage {
    db: Surreal<Any>,
}

impl SurrealStorage {
    /// Create a new SurrealStorage from an existing connection.
    pub fn new(db: Surreal<Any>) -> Self {
        Self { db }
    }

    /// Get a reference to the underlying SurrealDB client.
    pub fn db(&self) -> &Surreal<Any> {
        &self.db
    }
}

#[async_trait]
impl HealthCheck for SurrealStorage {
    async fn is_healthy(&self) -> bool {
        self.db.health().await.is_ok()
    }
}
