//! Per-user and per-room account data -- [`AccountDataStore`](crate::traits::AccountDataStore) implementation.
//!
//! Account data records are stored in the `account_data` table with a unique
//! index on `(user_id, room_id, data_type)`.  Global (non-room) account data
//! uses an empty string for `room_id` to keep the unique index simple.
//!
//! `INSERT ... ON DUPLICATE KEY UPDATE` provides atomic upsert semantics,
//! so setting the same key twice just overwrites the content.

use async_trait::async_trait;

use super::SurrealStorage;
use crate::traits::*;

#[async_trait]
impl AccountDataStore for SurrealStorage {
    async fn set_account_data(
        &self,
        user_id: &str,
        room_id: Option<&str>,
        data_type: &str,
        content: &serde_json::Value,
    ) -> StorageResult<()> {
        // Use empty string for global (no room) to make unique index work reliably
        let room_val = room_id.unwrap_or("").to_string();

        self.db()
            .query(
                "INSERT INTO account_data {
                    user_id: $uid,
                    room_id: $rid,
                    data_type: $dtype,
                    content: $content
                } ON DUPLICATE KEY UPDATE content = $content",
            )
            .bind(("uid", user_id.to_string()))
            .bind(("rid", room_val))
            .bind(("dtype", data_type.to_string()))
            .bind(("content", content.clone()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_account_data(
        &self,
        user_id: &str,
        room_id: Option<&str>,
        data_type: &str,
    ) -> StorageResult<serde_json::Value> {
        // Use empty string for global (no room) to match what set_account_data stores
        let room_val = room_id.unwrap_or("").to_string();

        let mut response = self
            .db()
            .query("SELECT content FROM account_data WHERE user_id = $uid AND room_id = $rid AND data_type = $dtype LIMIT 1")
            .bind(("uid", user_id.to_string()))
            .bind(("rid", room_val))
            .bind(("dtype", data_type.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .and_then(|row| row.get("content").cloned())
            .ok_or(StorageError::NotFound)
    }

    async fn get_all_account_data(
        &self,
        user_id: &str,
    ) -> StorageResult<Vec<(String, serde_json::Value)>> {
        let mut response = self
            .db()
            .query("SELECT data_type, content FROM account_data WHERE user_id = $uid AND room_id = '' AND string::starts_with(data_type, '_maelstrom.') = false")
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let dtype = row.get("data_type")?.as_str()?.to_string();
                let content = row.get("content")?.clone();
                Some((dtype, content))
            })
            .collect())
    }

    async fn get_all_room_account_data(
        &self,
        user_id: &str,
        room_id: &str,
    ) -> StorageResult<Vec<(String, serde_json::Value)>> {
        let mut response = self
            .db()
            .query("SELECT data_type, content FROM account_data WHERE user_id = $uid AND room_id = $rid AND string::starts_with(data_type, '_maelstrom.') = false")
            .bind(("uid", user_id.to_string()))
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let dtype = row.get("data_type")?.as_str()?.to_string();
                let content = row.get("content")?.clone();
                Some((dtype, content))
            })
            .collect())
    }

    async fn delete_account_data(
        &self,
        user_id: &str,
        room_id: Option<&str>,
        data_type: &str,
    ) -> StorageResult<()> {
        let room_val = room_id.unwrap_or("").to_string();

        self.db()
            .query("DELETE account_data WHERE user_id = $uid AND room_id = $rid AND data_type = $dtype")
            .bind(("uid", user_id.to_string()))
            .bind(("rid", room_val))
            .bind(("dtype", data_type.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }
}
