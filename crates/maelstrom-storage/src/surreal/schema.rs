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
