//! Application Service storage -- [`ApplicationServiceStore`](crate::traits::ApplicationServiceStore) implementation.
//!
//! Manages application service (bridge/bot) registration records in the
//! `appservice` table.  Each record holds authentication tokens, the AS
//! URL, namespace patterns (serialized as JSON strings), and protocol info.

use async_trait::async_trait;
use surrealdb::types::SurrealValue;
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

#[async_trait]
impl ApplicationServiceStore for SurrealStorage {
    async fn register_appservice(&self, record: &AppServiceRecord) -> StorageResult<()> {
        debug!(id = %record.id, "Registering application service");

        let user_ns = serde_json::to_string(&record.user_namespaces)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let alias_ns = serde_json::to_string(&record.alias_namespaces)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let protocols = serde_json::to_string(&record.protocols)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        self.db()
            .query(
                "DELETE appservice WHERE id = $id; \
                 INSERT INTO appservice { \
                     id: $id, \
                     url: $url, \
                     as_token: $as_token, \
                     hs_token: $hs_token, \
                     sender_localpart: $sender_localpart, \
                     user_namespaces: $user_namespaces, \
                     alias_namespaces: $alias_namespaces, \
                     rate_limited: $rate_limited, \
                     protocols: $protocols \
                 }",
            )
            .bind(("id", record.id.clone()))
            .bind(("url", record.url.clone()))
            .bind(("as_token", record.as_token.clone()))
            .bind(("hs_token", record.hs_token.clone()))
            .bind(("sender_localpart", record.sender_localpart.clone()))
            .bind(("user_namespaces", user_ns))
            .bind(("alias_namespaces", alias_ns))
            .bind(("rate_limited", record.rate_limited))
            .bind(("protocols", protocols))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn list_appservices(&self) -> StorageResult<Vec<AppServiceRecord>> {
        let mut response = self
            .db()
            .query("SELECT * FROM appservice")
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<AppServiceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter().map(|r| r.into_record()).collect()
    }

    async fn get_appservice(&self, id: &str) -> StorageResult<AppServiceRecord> {
        let mut response = self
            .db()
            .query("SELECT * FROM appservice WHERE id = $id LIMIT 1")
            .bind(("id", id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<AppServiceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .ok_or(StorageError::NotFound)?
            .into_record()
    }

    async fn get_appservice_by_token(&self, as_token: &str) -> StorageResult<AppServiceRecord> {
        let mut response = self
            .db()
            .query("SELECT * FROM appservice WHERE as_token = $token LIMIT 1")
            .bind(("token", as_token.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<AppServiceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .ok_or(StorageError::NotFound)?
            .into_record()
    }

    async fn delete_appservice(&self, id: &str) -> StorageResult<()> {
        debug!(id = %id, "Deleting application service");

        let mut response = self
            .db()
            .query("DELETE appservice WHERE id = $id RETURN BEFORE")
            .bind(("id", id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let deleted: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        if deleted.is_empty() {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }
}

/// Internal row type for deserializing from SurrealDB.
///
/// Namespace arrays and protocols are stored as JSON strings in the DB
/// and deserialized back into structured types on read.
#[derive(Debug, Clone, SurrealValue)]
struct AppServiceRow {
    id: String,
    url: String,
    as_token: String,
    hs_token: String,
    sender_localpart: String,
    user_namespaces: String,
    alias_namespaces: String,
    rate_limited: bool,
    protocols: String,
}

impl AppServiceRow {
    fn into_record(self) -> StorageResult<AppServiceRecord> {
        let user_namespaces: Vec<NamespaceRule> = serde_json::from_str(&self.user_namespaces)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let alias_namespaces: Vec<NamespaceRule> = serde_json::from_str(&self.alias_namespaces)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let protocols: Vec<String> = serde_json::from_str(&self.protocols)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(AppServiceRecord {
            id: self.id,
            url: self.url,
            as_token: self.as_token,
            hs_token: self.hs_token,
            sender_localpart: self.sender_localpart,
            user_namespaces,
            alias_namespaces,
            rate_limited: self.rate_limited,
            protocols,
        })
    }
}
