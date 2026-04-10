//! User account storage -- [`UserStore`](crate::traits::UserStore) implementation.
//!
//! Users are stored in the `user` table, keyed by `localpart`.  Profiles
//! (display name, avatar URL) are fields on the same record.
//!
//! `search_users` uses SurrealDB's full-text search index on the `display_name`
//! and `localpart` fields to power the `/user_directory/search` endpoint.

use async_trait::async_trait;
use surrealdb::types::{Datetime, RecordId, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Content for creating a user record.
#[derive(Debug, Clone, SurrealValue)]
struct UserInput {
    localpart: String,
    password_hash: Option<String>,
    is_admin: bool,
    is_guest: bool,
    is_deactivated: bool,
}

/// Row returned when reading a user record.
#[derive(Debug, Clone, SurrealValue)]
struct UserRow {
    id: RecordId,
    localpart: String,
    password_hash: Option<String>,
    is_admin: bool,
    is_guest: bool,
    is_deactivated: bool,
    created_at: Datetime,
}

impl UserRow {
    fn into_record(self) -> UserRecord {
        UserRecord {
            localpart: self.localpart,
            password_hash: self.password_hash,
            is_admin: self.is_admin,
            is_guest: self.is_guest,
            is_deactivated: self.is_deactivated,
            created_at: self.created_at.into_inner(),
        }
    }
}

/// Row returned when reading a profile record.
#[derive(Debug, Clone, SurrealValue)]
struct ProfileRow {
    display_name: Option<String>,
    avatar_url: Option<String>,
}

/// Content for creating a profile record.
#[derive(Debug, Clone, SurrealValue)]
struct ProfileInput {
    user: RecordId,
    display_name: Option<String>,
    avatar_url: Option<String>,
}

#[async_trait]
impl UserStore for SurrealStorage {
    async fn create_user(&self, user: &UserRecord) -> StorageResult<()> {
        debug!(localpart = %user.localpart, "Creating user");

        let user_rid = RecordId::new("user", user.localpart.as_str());

        let input = UserInput {
            localpart: user.localpart.clone(),
            password_hash: user.password_hash.clone(),
            is_admin: user.is_admin,
            is_guest: user.is_guest,
            is_deactivated: user.is_deactivated,
        };

        // Create user record with localpart as the record key
        let _: Option<serde_json::Value> = self
            .db()
            .create(user_rid.clone())
            .content(input)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("unique") {
                    StorageError::Duplicate(user.localpart.clone())
                } else {
                    StorageError::Query(msg)
                }
            })?;

        // Create associated profile record
        let profile = ProfileInput {
            user: user_rid,
            display_name: None,
            avatar_url: None,
        };

        let _: Option<serde_json::Value> = self
            .db()
            .create("profile")
            .content(profile)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_user(&self, localpart: &str) -> StorageResult<UserRecord> {
        let rid = RecordId::new("user", localpart);

        let result: Option<UserRow> = self
            .db()
            .select(rid)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        result
            .map(|row| row.into_record())
            .ok_or(StorageError::NotFound)
    }

    async fn user_exists(&self, localpart: &str) -> StorageResult<bool> {
        let rid = RecordId::new("user", localpart);

        let result: Option<serde_json::Value> = self
            .db()
            .select(rid)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(result.is_some())
    }

    async fn set_password_hash(&self, localpart: &str, hash: &str) -> StorageResult<()> {
        let rid = RecordId::new("user", localpart);
        let h = hash.to_string();

        let mut response = self
            .db()
            .query("UPDATE $rid SET password_hash = $hash")
            .bind(("rid", rid))
            .bind(("hash", h))
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

    async fn set_deactivated(&self, localpart: &str, deactivated: bool) -> StorageResult<()> {
        let rid = RecordId::new("user", localpart);

        let mut response = self
            .db()
            .query("UPDATE $rid SET is_deactivated = $val")
            .bind(("rid", rid))
            .bind(("val", deactivated))
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

    async fn set_admin(&self, localpart: &str, is_admin: bool) -> StorageResult<()> {
        let rid = RecordId::new("user", localpart);

        let mut response = self
            .db()
            .query("UPDATE $rid SET is_admin = $val")
            .bind(("rid", rid))
            .bind(("val", is_admin))
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

    async fn count_users(&self) -> StorageResult<u64> {
        let mut response = self
            .db()
            .query("SELECT count() AS total FROM user GROUP ALL")
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .first()
            .and_then(|v| v.get("total"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0))
    }

    async fn get_profile(&self, localpart: &str) -> StorageResult<ProfileRecord> {
        let user_rid = RecordId::new("user", localpart);

        let mut response = self
            .db()
            .query("SELECT display_name, avatar_url FROM profile WHERE user = $user_rid")
            .bind(("user_rid", user_rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ProfileRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|row| ProfileRecord {
                display_name: row.display_name,
                avatar_url: row.avatar_url,
            })
            .ok_or(StorageError::NotFound)
    }

    async fn set_display_name(&self, localpart: &str, name: Option<&str>) -> StorageResult<()> {
        let user_rid = RecordId::new("user", localpart);
        let n = name.map(|s| s.to_string());

        self.db()
            .query("UPDATE profile SET display_name = $name WHERE user = $user_rid")
            .bind(("user_rid", user_rid))
            .bind(("name", n))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn set_avatar_url(&self, localpart: &str, url: Option<&str>) -> StorageResult<()> {
        let user_rid = RecordId::new("user", localpart);
        let u = url.map(|s| s.to_string());

        self.db()
            .query("UPDATE profile SET avatar_url = $url WHERE user = $user_rid")
            .bind(("user_rid", user_rid))
            .bind(("url", u))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn search_users(
        &self,
        search_term: &str,
        limit: usize,
    ) -> StorageResult<Vec<(String, Option<String>, Option<String>)>> {
        // Detect mxid searches (exact match on localpart)
        let is_mxid = search_term.starts_with('@') && search_term.contains(':');
        let cleaned = search_term
            .trim_start_matches('@')
            .split(':')
            .next()
            .unwrap_or(search_term);
        let term_lower = cleaned.to_lowercase();

        // Get all users with their profiles via separate queries
        let mut response = self
            .db()
            .query("SELECT localpart FROM user LIMIT 1000")
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let localpart = match row.get("localpart").and_then(|v| v.as_str()) {
                Some(lp) => lp.to_string(),
                None => continue,
            };

            // Get profile for this user
            let profile = self.get_profile(&localpart).await.ok();
            let display_name = profile.as_ref().and_then(|p| p.display_name.clone());
            let avatar_url = profile.as_ref().and_then(|p| p.avatar_url.clone());

            // Check if localpart or display_name matches the search term
            let localpart_lower = localpart.to_lowercase();
            let localpart_matches = if is_mxid {
                localpart_lower == term_lower
            } else {
                localpart_lower.contains(&term_lower)
            };
            // When the search term is a full mxid (@user:server), only match
            // on exact localpart — don't also fuzzy-match display names, which
            // would return unrelated users whose names happen to contain the
            // localpart substring.
            let name_matches = if is_mxid {
                false
            } else {
                display_name
                    .as_ref()
                    .map(|n| n.to_lowercase().contains(&term_lower))
                    .unwrap_or(false)
            };

            if localpart_matches || name_matches {
                results.push((localpart, display_name, avatar_url));
                if results.len() >= limit {
                    break;
                }
            }
        }

        Ok(results)
    }
}
