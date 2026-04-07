use tracing::info;

use super::SurrealStorage;
use crate::traits::StorageError;

/// Bootstrap the SurrealDB schema.
///
/// This is idempotent — safe to run on every startup.
/// Uses DEFINE TABLE/FIELD with OVERWRITE to ensure schema is current.
pub async fn bootstrap(storage: &SurrealStorage) -> Result<(), StorageError> {
    info!("Bootstrapping SurrealDB schema");

    let db = storage.db();

    db.query(SCHEMA)
        .await
        .map_err(|e| StorageError::Query(format!("Schema bootstrap failed: {e}")))?;

    info!("Schema bootstrap complete");
    Ok(())
}

/// The complete SurrealQL schema definition.
///
/// Tables and fields defined here map to the storage traits.
/// Graph relations (TYPE RELATION) are used for Matrix's relational data model.
const SCHEMA: &str = r#"
-- =============================================================
-- Users
-- =============================================================
DEFINE TABLE IF NOT EXISTS user SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS localpart   ON TABLE user TYPE string;
DEFINE FIELD IF NOT EXISTS password_hash ON TABLE user TYPE option<string>;
DEFINE FIELD IF NOT EXISTS is_admin     ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS is_guest     ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS is_deactivated ON TABLE user TYPE bool DEFAULT false;
DEFINE FIELD IF NOT EXISTS created_at   ON TABLE user TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_user_localpart ON TABLE user FIELDS localpart UNIQUE;

-- =============================================================
-- User Profiles
-- =============================================================
DEFINE TABLE IF NOT EXISTS profile SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS user         ON TABLE profile TYPE record<user>;
DEFINE FIELD IF NOT EXISTS display_name ON TABLE profile TYPE option<string>;
DEFINE FIELD IF NOT EXISTS avatar_url   ON TABLE profile TYPE option<string>;

DEFINE INDEX IF NOT EXISTS idx_profile_user ON TABLE profile FIELDS user UNIQUE;

-- =============================================================
-- Devices
-- =============================================================
DEFINE TABLE IF NOT EXISTS device SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS device_id     ON TABLE device TYPE string;
DEFINE FIELD IF NOT EXISTS user          ON TABLE device TYPE record<user>;
DEFINE FIELD IF NOT EXISTS display_name  ON TABLE device TYPE option<string>;
DEFINE FIELD IF NOT EXISTS access_token  ON TABLE device TYPE string;
DEFINE FIELD IF NOT EXISTS created_at    ON TABLE device TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_device_access_token ON TABLE device FIELDS access_token UNIQUE;
DEFINE INDEX IF NOT EXISTS idx_device_user_device  ON TABLE device FIELDS user, device_id UNIQUE;

-- =============================================================
-- Rooms (stub for Phase 3)
-- =============================================================
DEFINE TABLE IF NOT EXISTS room SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS room_id      ON TABLE room TYPE string;
DEFINE FIELD IF NOT EXISTS version      ON TABLE room TYPE string DEFAULT "11";
DEFINE FIELD IF NOT EXISTS created_at   ON TABLE room TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_room_room_id ON TABLE room FIELDS room_id UNIQUE;

-- =============================================================
-- Membership relation: user -> membership -> room (stub for Phase 3)
-- =============================================================
DEFINE TABLE IF NOT EXISTS membership TYPE RELATION IN user OUT room SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS membership ON TABLE membership TYPE string;  -- join, invite, leave, ban, knock
DEFINE FIELD IF NOT EXISTS since      ON TABLE membership TYPE datetime DEFAULT time::now();

-- =============================================================
-- Events (stub for Phase 3)
-- =============================================================
DEFINE TABLE IF NOT EXISTS event SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS event_id       ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS room           ON TABLE event TYPE record<room>;
DEFINE FIELD IF NOT EXISTS sender         ON TABLE event TYPE record<user>;
DEFINE FIELD IF NOT EXISTS event_type     ON TABLE event TYPE string;
DEFINE FIELD IF NOT EXISTS state_key      ON TABLE event TYPE option<string>;
DEFINE FIELD IF NOT EXISTS content        ON TABLE event TYPE object;
DEFINE FIELD IF NOT EXISTS origin_server_ts ON TABLE event TYPE int;
DEFINE FIELD IF NOT EXISTS depth          ON TABLE event TYPE int DEFAULT 0;
DEFINE FIELD IF NOT EXISTS created_at     ON TABLE event TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_event_event_id ON TABLE event FIELDS event_id UNIQUE;

-- Event DAG edges (stub for Phase 3)
DEFINE TABLE IF NOT EXISTS event_edge TYPE RELATION IN event OUT event SCHEMAFULL;

-- =============================================================
-- Server signing keys
-- =============================================================
DEFINE TABLE IF NOT EXISTS server_key SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS key_id      ON TABLE server_key TYPE string;
DEFINE FIELD IF NOT EXISTS algorithm   ON TABLE server_key TYPE string DEFAULT "ed25519";
DEFINE FIELD IF NOT EXISTS public_key  ON TABLE server_key TYPE string;
DEFINE FIELD IF NOT EXISTS private_key ON TABLE server_key TYPE string;
DEFINE FIELD IF NOT EXISTS valid_until ON TABLE server_key TYPE datetime;
DEFINE FIELD IF NOT EXISTS created_at  ON TABLE server_key TYPE datetime DEFAULT time::now();

DEFINE INDEX IF NOT EXISTS idx_server_key_key_id ON TABLE server_key FIELDS key_id UNIQUE;
"#;
