//! Schema definitions and migrations.
//!
//! The canonical schema lives in `db/schema.surql` at the repository root and
//! is embedded into the binary at compile time with `include_str!`.  This means
//! the server binary is self-contained -- no external SQL files to deploy.
//!
//! [`bootstrap`] executes the full schema on every startup.  Every statement
//! uses `DEFINE ... IF NOT EXISTS`, so re-running it against an existing
//! database is a no-op for tables/indexes that already exist.  This makes
//! rolling deployments safe without a separate migration tool.

use tracing::info;

use super::SurrealStorage;
use crate::traits::StorageError;

/// The schema is embedded at compile time from the external .surql file.
/// This keeps the schema as a reviewable, standalone file while still
/// bundling it into the binary for zero-config deployment.
const SCHEMA: &str = include_str!("../../../../db/schema.surql");

/// Bootstrap the SurrealDB schema.
///
/// This is idempotent — safe to run on every startup.
/// Uses `IF NOT EXISTS` throughout so existing data is never dropped.
pub async fn bootstrap(storage: &SurrealStorage) -> Result<(), StorageError> {
    info!("Bootstrapping SurrealDB schema");

    storage
        .db()
        .query(SCHEMA)
        .await
        .map_err(|e| StorageError::Query(format!("Schema bootstrap failed: {e}")))?;

    info!("Schema bootstrap complete");
    Ok(())
}
