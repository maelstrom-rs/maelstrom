//! Database connection and initialization.
//!
//! Provides [`SurrealConfig`] (endpoint, namespace, database, credentials) and
//! the [`SurrealStorage::connect`] constructor that:
//!
//! 1. Opens a connection to the configured endpoint (WebSocket, in-memory, or
//!    RocksDB embedded).
//! 2. Authenticates as root.
//! 3. Selects the target namespace and database.
//! 4. Runs the schema bootstrap (see [`super::schema`]).
//!
//! The returned [`SurrealStorage`] is ready to use immediately.

use surrealdb::engine::any;
use surrealdb::opt::auth::Root;
use tracing::info;

use super::SurrealStorage;
use super::schema;
use crate::traits::StorageError;

/// Configuration for connecting to SurrealDB.
#[derive(Debug, Clone)]
pub struct SurrealConfig {
    /// Connection endpoint, e.g. `ws://localhost:8000`, `mem://`, `rocksdb://data/db`
    pub endpoint: String,
    /// Namespace to use
    pub namespace: String,
    /// Database to use
    pub database: String,
    /// Root username (for authentication)
    pub username: String,
    /// Root password (for authentication)
    pub password: String,
}

impl Default for SurrealConfig {
    fn default() -> Self {
        Self {
            endpoint: "ws://localhost:8000".to_string(),
            namespace: "maelstrom".to_string(),
            database: "maelstrom".to_string(),
            username: "root".to_string(),
            password: "root".to_string(),
        }
    }
}

impl SurrealStorage {
    /// Connect to SurrealDB and bootstrap the schema.
    pub async fn connect(config: &SurrealConfig) -> Result<Self, StorageError> {
        info!(endpoint = %config.endpoint, "Connecting to SurrealDB");

        let db = any::connect(&config.endpoint)
            .await
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        // Authenticate if not using an embedded engine
        let is_embedded = config.endpoint.starts_with("mem://")
            || config.endpoint == "memory"
            || config.endpoint.starts_with("surrealkv://")
            || config.endpoint.starts_with("rocksdb://")
            || config.endpoint.starts_with("file://");
        if !is_embedded {
            db.signin(Root {
                username: config.username.clone(),
                password: config.password.clone(),
            })
            .await
            .map_err(|e| StorageError::Connection(format!("Authentication failed: {e}")))?;
        }

        db.use_ns(&config.namespace)
            .use_db(&config.database)
            .await
            .map_err(|e| StorageError::Connection(format!("Failed to select ns/db: {e}")))?;

        info!("Connected to SurrealDB");

        let storage = Self::new(db);

        // Bootstrap schema
        schema::bootstrap(&storage).await?;

        Ok(storage)
    }
}
