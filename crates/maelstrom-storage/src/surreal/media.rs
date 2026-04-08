use async_trait::async_trait;
use surrealdb::types::{Datetime, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Input for creating a media metadata record.
#[derive(Debug, Clone, SurrealValue)]
struct MediaInput {
    media_id: String,
    server_name: String,
    user_id: String,
    content_type: String,
    content_length: i64,
    filename: Option<String>,
    s3_key: String,
    quarantined: bool,
}

/// Row returned when reading a media record.
#[derive(Debug, Clone, SurrealValue)]
struct MediaRow {
    media_id: String,
    server_name: String,
    user_id: String,
    content_type: String,
    content_length: i64,
    filename: Option<String>,
    s3_key: String,
    quarantined: bool,
    created_at: Datetime,
}

impl MediaRow {
    fn into_record(self) -> MediaRecord {
        MediaRecord {
            media_id: self.media_id,
            server_name: self.server_name,
            user_id: self.user_id,
            content_type: self.content_type,
            content_length: self.content_length as u64,
            filename: self.filename,
            s3_key: self.s3_key,
            quarantined: self.quarantined,
            created_at: self.created_at.into_inner(),
        }
    }
}

#[async_trait]
impl MediaStore for SurrealStorage {
    async fn store_media(&self, media: &MediaRecord) -> StorageResult<()> {
        debug!(media_id = %media.media_id, server_name = %media.server_name, "Storing media metadata");

        let input = MediaInput {
            media_id: media.media_id.clone(),
            server_name: media.server_name.clone(),
            user_id: media.user_id.clone(),
            content_type: media.content_type.clone(),
            content_length: media.content_length as i64,
            filename: media.filename.clone(),
            s3_key: media.s3_key.clone(),
            quarantined: media.quarantined,
        };

        let _: Option<serde_json::Value> =
            self.db()
                .create("media")
                .content(input)
                .await
                .map_err(|e| {
                    let msg = e.to_string();
                    if msg.contains("already exists") || msg.contains("unique") {
                        StorageError::Duplicate(media.media_id.clone())
                    } else {
                        StorageError::Query(msg)
                    }
                })?;

        Ok(())
    }

    async fn get_media(&self, server_name: &str, media_id: &str) -> StorageResult<MediaRecord> {
        let mut response = self
            .db()
            .query("SELECT * FROM media WHERE server_name = $server AND media_id = $mid LIMIT 1")
            .bind(("server", server_name.to_string()))
            .bind(("mid", media_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<MediaRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|row| row.into_record())
            .ok_or(StorageError::NotFound)
    }

    async fn list_user_media(
        &self,
        user_id: &str,
        limit: usize,
    ) -> StorageResult<Vec<MediaRecord>> {
        let mut response = self
            .db()
            .query("SELECT * FROM media WHERE user_id = $uid ORDER BY created_at DESC LIMIT $lim")
            .bind(("uid", user_id.to_string()))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<MediaRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_record()).collect())
    }

    async fn set_media_quarantined(
        &self,
        server_name: &str,
        media_id: &str,
        quarantined: bool,
    ) -> StorageResult<()> {
        let mut response = self
            .db()
            .query(
                "UPDATE media SET quarantined = $q WHERE server_name = $server AND media_id = $mid",
            )
            .bind(("server", server_name.to_string()))
            .bind(("mid", media_id.to_string()))
            .bind(("q", quarantined))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let updated: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        if updated.is_empty() {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    async fn delete_media(&self, server_name: &str, media_id: &str) -> StorageResult<()> {
        let mut response = self
            .db()
            .query(
                "DELETE FROM media WHERE server_name = $server AND media_id = $mid RETURN BEFORE",
            )
            .bind(("server", server_name.to_string()))
            .bind(("mid", media_id.to_string()))
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

    async fn list_media_before(
        &self,
        before: chrono::DateTime<chrono::Utc>,
        limit: usize,
    ) -> StorageResult<Vec<MediaRecord>> {
        let before_dt = surrealdb::types::Datetime::from(before);

        let mut response = self
            .db()
            .query(
                "SELECT * FROM media WHERE created_at < $before ORDER BY created_at ASC LIMIT $lim",
            )
            .bind(("before", before_dt))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<MediaRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_record()).collect())
    }
}
