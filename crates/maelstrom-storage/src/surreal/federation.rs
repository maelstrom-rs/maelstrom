//! Federation-related storage -- [`FederationKeyStore`](crate::traits::FederationKeyStore) implementation.
//!
//! Manages three concerns:
//!
//! 1. **Server signing keys** (`server_key` table) -- this server's own
//!    ed25519 key pairs used to sign outgoing federation requests and events.
//! 2. **Remote server keys** (`remote_key` table) -- cached public keys
//!    fetched from other homeservers, used to verify incoming signatures.
//! 3. **Transaction deduplication** (`federation_txn` table) -- tracks
//!    `(origin, txn_id)` pairs so that replayed federation transactions are
//!    rejected.

use async_trait::async_trait;
use surrealdb::types::{Datetime, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

#[derive(Debug, Clone, SurrealValue)]
struct ServerKeyRow {
    key_id: String,
    algorithm: String,
    public_key: String,
    private_key: String,
    valid_until: Datetime,
}

#[derive(Debug, Clone, SurrealValue)]
struct RemoteKeyRow {
    server_name: String,
    key_id: String,
    public_key: String,
    valid_until: Datetime,
}

#[async_trait]
impl FederationKeyStore for SurrealStorage {
    async fn store_server_key(&self, key: &ServerKeyRecord) -> StorageResult<()> {
        debug!(key_id = %key.key_id, "Storing server signing key");

        self.db()
            .query(
                "CREATE server_key SET \
                 key_id = $key_id, \
                 algorithm = $algo, \
                 public_key = $pub_key, \
                 private_key = $priv_key, \
                 valid_until = $valid_until",
            )
            .bind(("key_id", key.key_id.clone()))
            .bind(("algo", key.algorithm.clone()))
            .bind(("pub_key", key.public_key.clone()))
            .bind(("priv_key", key.private_key.clone()))
            .bind(("valid_until", Datetime::from(key.valid_until)))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_server_key(&self, key_id: &str) -> StorageResult<ServerKeyRecord> {
        let mut response = self
            .db()
            .query("SELECT * FROM server_key WHERE key_id = $kid LIMIT 1")
            .bind(("kid", key_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ServerKeyRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|r| ServerKeyRecord {
                key_id: r.key_id,
                algorithm: r.algorithm,
                public_key: r.public_key,
                private_key: r.private_key,
                valid_until: r.valid_until.into_inner(),
            })
            .ok_or(StorageError::NotFound)
    }

    async fn get_active_server_keys(&self) -> StorageResult<Vec<ServerKeyRecord>> {
        let mut response = self
            .db()
            .query("SELECT * FROM server_key ORDER BY created_at DESC")
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ServerKeyRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| ServerKeyRecord {
                key_id: r.key_id,
                algorithm: r.algorithm,
                public_key: r.public_key,
                private_key: r.private_key,
                valid_until: r.valid_until.into_inner(),
            })
            .collect())
    }

    async fn store_remote_server_keys(&self, keys: &[RemoteKeyRecord]) -> StorageResult<()> {
        for key in keys {
            self.db()
                .query(
                    "DELETE remote_server_key WHERE server_name = $sn AND key_id = $kid; \
                     CREATE remote_server_key SET \
                     server_name = $sn, key_id = $kid, public_key = $pk, valid_until = $vu",
                )
                .bind(("sn", key.server_name.clone()))
                .bind(("kid", key.key_id.clone()))
                .bind(("pk", key.public_key.clone()))
                .bind(("vu", Datetime::from(key.valid_until)))
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;
        }
        Ok(())
    }

    async fn get_remote_server_keys(
        &self,
        server_name: &str,
    ) -> StorageResult<Vec<RemoteKeyRecord>> {
        let mut response = self
            .db()
            .query("SELECT * FROM remote_server_key WHERE server_name = $sn")
            .bind(("sn", server_name.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RemoteKeyRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| RemoteKeyRecord {
                server_name: r.server_name,
                key_id: r.key_id,
                public_key: r.public_key,
                valid_until: r.valid_until.into_inner(),
            })
            .collect())
    }

    async fn store_federation_txn(&self, origin: &str, txn_id: &str) -> StorageResult<()> {
        self.db()
            .query("CREATE federation_txn SET origin = $origin, txn_id = $tid")
            .bind(("origin", origin.to_string()))
            .bind(("tid", txn_id.to_string()))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("unique") {
                    StorageError::Duplicate(format!("{origin}:{txn_id}"))
                } else {
                    StorageError::Query(msg)
                }
            })?;
        Ok(())
    }

    async fn has_federation_txn(&self, origin: &str, txn_id: &str) -> StorageResult<bool> {
        let mut response = self
            .db()
            .query("SELECT * FROM federation_txn WHERE origin = $origin AND txn_id = $tid LIMIT 1")
            .bind(("origin", origin.to_string()))
            .bind(("tid", txn_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(!rows.is_empty())
    }
}
