pub mod connection;
pub mod schema;
mod users;
mod devices;
mod rooms;
mod events;
mod receipts;
mod keys;
mod account_data;
mod federation;
mod media;
mod relations;

use async_trait::async_trait;
use surrealdb::Surreal;
use surrealdb::engine::any::Any;

use crate::traits::*;

/// SurrealDB-backed storage implementation.
///
/// This is the primary storage backend for Maelstrom.
/// It wraps a SurrealDB client and implements all storage traits.
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
