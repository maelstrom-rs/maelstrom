//! Room metadata and membership storage -- [`RoomStore`](crate::traits::RoomStore) implementation.
//!
//! Rooms are stored in the `room` table.  Membership is modeled as a SurrealDB
//! graph edge: `user ->member_of-> room` with a `membership` field on the edge
//! (`join`, `invite`, `leave`, `ban`).  This enables efficient queries like
//! "all rooms a user has joined" or "all members of a room" via graph
//! traversal rather than scanning a flat membership table.
//!
//! Room aliases are stored in a separate `room_alias` table.  The public room
//! directory (`get_public_rooms`) supports optional keyword filtering and
//! cursor-based pagination.
//!
//! Room upgrades are modeled as `room ->upgrades_to-> room` graph edges,
//! allowing `get_room_predecessors` to walk the chain backward in a single
//! recursive traversal.

use async_trait::async_trait;
use surrealdb::types::{RecordId, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Extract localpart from a Matrix user ID (e.g. `@alice:hs1` -> `alice`)
/// and create a `user` RecordId.
fn user_rid_from_matrix_id(user_id: &str) -> RecordId {
    let localpart = user_id
        .trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or(user_id);
    RecordId::new("user", localpart)
}

/// Create a `room` RecordId from a Matrix room_id string.
fn room_rid(room_id: &str) -> RecordId {
    RecordId::new("room", room_id)
}

#[derive(Debug, Clone, SurrealValue)]
struct RoomRow {
    room_id: String,
    version: String,
    creator: String,
    is_direct: bool,
}

impl RoomRow {
    fn into_record(self) -> RoomRecord {
        RoomRecord {
            room_id: self.room_id,
            version: self.version,
            creator: self.creator,
            is_direct: self.is_direct,
        }
    }
}

#[derive(Debug, Clone, SurrealValue)]
struct MembershipRow {
    membership: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct RoomIdRow {
    room_id: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct UserIdRow {
    user_id: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct AliasRow {
    alias: String,
    room_id: String,
    creator: String,
}

#[async_trait]
impl RoomStore for SurrealStorage {
    async fn create_room(&self, room: &RoomRecord) -> StorageResult<()> {
        debug!(room_id = %room.room_id, "Creating room");

        let rid = room_rid(&room.room_id);

        let mut response = self
            .db()
            .query(
                "INSERT INTO room { id: $rid, room_id: $room_id, version: $ver, creator: $creator, is_direct: $direct } \
                 ON DUPLICATE KEY UPDATE room_id = $room_id",
            )
            .bind(("rid", rid))
            .bind(("room_id", room.room_id.clone()))
            .bind(("ver", room.version.clone()))
            .bind(("creator", room.creator.clone()))
            .bind(("direct", room.is_direct))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("unique") {
                    StorageError::Duplicate(room.room_id.clone())
                } else {
                    StorageError::Query(msg)
                }
            })?;

        let _: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_room(&self, room_id: &str) -> StorageResult<RoomRecord> {
        let mut response = self
            .db()
            .query("SELECT room_id, version, creator, is_direct FROM room WHERE room_id = $rid")
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RoomRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|row| row.into_record())
            .ok_or(StorageError::NotFound)
    }

    async fn set_membership(
        &self,
        user_id: &str,
        room_id: &str,
        membership: &str,
    ) -> StorageResult<()> {
        debug!(user_id = %user_id, room_id = %room_id, membership = %membership, "Setting membership");

        let user_record = user_rid_from_matrix_id(user_id);
        let room_record = room_rid(room_id);

        // Delete existing edge then create new one (upsert pattern for RELATION tables)
        self.db()
            .query("DELETE member_of WHERE in = $user_rid AND out = $room_rid")
            .bind(("user_rid", user_record.clone()))
            .bind(("room_rid", room_record.clone()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let mut relate_resp = self.db()
            .query(
                "RELATE $user_rid->member_of->$room_rid SET \
                     membership = $mem, user_id = $uid, room_id = $rid, updated_at = time::now()",
            )
            .bind(("user_rid", user_record))
            .bind(("room_rid", room_record))
            .bind(("mem", membership.to_string()))
            .bind(("uid", user_id.to_string()))
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| {
                tracing::error!(user_id = %user_id, room_id = %room_id, error = %e, "RELATE member_of failed");
                StorageError::Query(e.to_string())
            })?;

        // Check if the RELATE actually created a record
        let created: Vec<serde_json::Value> = relate_resp.take(0).unwrap_or_default();
        if created.is_empty() {
            tracing::warn!(user_id = %user_id, room_id = %room_id, "RELATE member_of returned empty result — edge may not have been created");
        }

        Ok(())
    }

    async fn get_membership(&self, user_id: &str, room_id: &str) -> StorageResult<String> {
        let mut response = self
            .db()
            .query("SELECT membership FROM member_of WHERE in = $user_rid AND out = $room_rid")
            .bind(("user_rid", user_rid_from_matrix_id(user_id)))
            .bind(("room_rid", room_rid(room_id)))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<MembershipRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|row| row.membership)
            .ok_or(StorageError::NotFound)
    }

    async fn get_joined_rooms(&self, user_id: &str) -> StorageResult<Vec<String>> {
        let mut response = self
            .db()
            .query("SELECT room_id FROM member_of WHERE in = $user_rid AND membership = 'join'")
            .bind(("user_rid", user_rid_from_matrix_id(user_id)))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RoomIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.room_id).collect())
    }

    async fn get_invited_rooms(&self, user_id: &str) -> StorageResult<Vec<String>> {
        let mut response = self
            .db()
            .query("SELECT room_id FROM member_of WHERE in = $user_rid AND membership = 'invite'")
            .bind(("user_rid", user_rid_from_matrix_id(user_id)))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RoomIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.room_id).collect())
    }

    async fn get_left_rooms(&self, user_id: &str) -> StorageResult<Vec<String>> {
        let mut response = self
            .db()
            .query("SELECT room_id FROM member_of WHERE in = $user_rid AND membership = 'leave'")
            .bind(("user_rid", user_rid_from_matrix_id(user_id)))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RoomIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.room_id).collect())
    }

    async fn get_room_members(
        &self,
        room_id: &str,
        membership: &str,
    ) -> StorageResult<Vec<String>> {
        let mut response = self
            .db()
            .query("SELECT user_id FROM member_of WHERE out = $room_rid AND membership = $state")
            .bind(("room_rid", room_rid(room_id)))
            .bind(("state", membership.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<UserIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.user_id).collect())
    }

    async fn set_room_alias(&self, alias: &str, room_id: &str, creator: &str) -> StorageResult<()> {
        debug!(alias = %alias, room_id = %room_id, "Setting room alias");

        self.db()
            .query(
                "BEGIN TRANSACTION; \
                 DELETE room_alias WHERE alias = $alias; \
                 CREATE room_alias SET alias = $alias, room_id = $rid, creator = $creator; \
                 COMMIT TRANSACTION;",
            )
            .bind(("alias", alias.to_string()))
            .bind(("rid", room_id.to_string()))
            .bind(("creator", creator.to_string()))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("unique") {
                    StorageError::Duplicate(alias.to_string())
                } else {
                    StorageError::Query(msg)
                }
            })?;

        Ok(())
    }

    async fn get_room_alias(&self, alias: &str) -> StorageResult<String> {
        let mut response = self
            .db()
            .query("SELECT room_id FROM room_alias WHERE alias = $alias")
            .bind(("alias", alias.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RoomIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|r| r.room_id)
            .ok_or(StorageError::NotFound)
    }

    async fn delete_room_alias(&self, alias: &str) -> StorageResult<()> {
        let mut response = self
            .db()
            .query("DELETE room_alias WHERE alias = $alias RETURN BEFORE")
            .bind(("alias", alias.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        if rows.is_empty() {
            return Err(StorageError::NotFound);
        }

        Ok(())
    }

    async fn get_room_alias_creator(&self, alias: &str) -> StorageResult<String> {
        let mut response = self
            .db()
            .query("SELECT creator FROM room_alias WHERE alias = $alias")
            .bind(("alias", alias.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.first()
            .and_then(|r| r.get("creator"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or(StorageError::NotFound)
    }

    async fn get_room_aliases(&self, room_id: &str) -> StorageResult<Vec<String>> {
        let mut response = self
            .db()
            .query("SELECT * FROM room_alias WHERE room_id = $rid")
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<AliasRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.alias).collect())
    }

    async fn set_room_visibility(&self, room_id: &str, visibility: &str) -> StorageResult<()> {
        debug!(room_id = %room_id, visibility = %visibility, "Setting room visibility");

        self.db()
            .query("UPDATE room SET visibility = $vis WHERE room_id = $rid")
            .bind(("rid", room_id.to_string()))
            .bind(("vis", visibility.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_public_rooms(
        &self,
        limit: usize,
        since: Option<&str>,
        filter: Option<&str>,
    ) -> StorageResult<(Vec<PublicRoom>, usize)> {
        let mut response = self
            .db()
            .query("SELECT room_id FROM room WHERE visibility = 'public'")
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let all_rooms: Vec<RoomIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let total = all_rooms.len();
        let start = since.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);

        let mut public_rooms = Vec::new();
        for room_row in all_rooms.into_iter().skip(start) {
            let rid = room_row.room_id;

            // Fetch all current state for this room in one query
            let mut state_resp = self
                .db()
                .query("SELECT event_type, event_id FROM room_state WHERE room_id = $rid AND state_key = '' AND event_type IN ['m.room.name', 'm.room.topic', 'm.room.canonical_alias', 'm.room.avatar']")
                .bind(("rid", rid.clone()))
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;

            let state_rows: Vec<serde_json::Value> = state_resp.take(0).unwrap_or_default();

            let mut name = None;
            let mut topic = None;
            let mut canonical_alias = None;
            let mut avatar_url = None;

            for row in &state_rows {
                let event_type = row.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
                let event_id = row.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
                if event_id.is_empty() {
                    continue;
                }

                // Fetch the event content
                let mut ev_resp = self
                    .db()
                    .query("SELECT content FROM event WHERE event_id = $eid LIMIT 1")
                    .bind(("eid", event_id.to_string()))
                    .await
                    .map_err(|e| StorageError::Query(e.to_string()))?;

                let ev_rows: Vec<serde_json::Value> = ev_resp.take(0).unwrap_or_default();
                let content = ev_rows.first().and_then(|v| v.get("content"));

                match event_type {
                    "m.room.name" => {
                        name = content
                            .and_then(|c| c.get("name"))
                            .and_then(|n| n.as_str())
                            .map(|s| s.to_string());
                    }
                    "m.room.topic" => {
                        topic = content
                            .and_then(|c| c.get("topic"))
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string());
                    }
                    "m.room.canonical_alias" => {
                        canonical_alias = content
                            .and_then(|c| c.get("alias"))
                            .and_then(|a| a.as_str())
                            .map(|s| s.to_string());
                    }
                    "m.room.avatar" => {
                        avatar_url = content
                            .and_then(|c| c.get("url"))
                            .and_then(|u| u.as_str())
                            .map(|s| s.to_string());
                    }
                    _ => {}
                }
            }

            // Count joined members
            let mut count_resp = self
                .db()
                .query("SELECT count() AS total FROM member_of WHERE out = $room_rid AND membership = 'join' GROUP ALL")
                .bind(("room_rid", room_rid(&rid)))
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;

            let count_rows: Vec<serde_json::Value> = count_resp.take(0).unwrap_or_default();
            let num_joined = count_rows
                .first()
                .and_then(|v| v.get("total"))
                .and_then(|t| t.as_u64())
                .unwrap_or(0) as usize;

            if let Some(f) = filter {
                let f_lower = f.to_lowercase();
                let matches = name
                    .as_deref()
                    .map(|n| n.to_lowercase().contains(&f_lower))
                    .unwrap_or(false)
                    || topic
                        .as_deref()
                        .map(|t| t.to_lowercase().contains(&f_lower))
                        .unwrap_or(false);
                if !matches {
                    continue;
                }
            }

            public_rooms.push(PublicRoom {
                room_id: rid,
                name,
                topic,
                canonical_alias,
                avatar_url,
                num_joined_members: num_joined,
                world_readable: false,
                guest_can_join: false,
            });

            if public_rooms.len() >= limit {
                break;
            }
        }

        Ok((public_rooms, total))
    }

    async fn forget_room(&self, user_id: &str, room_id: &str) -> StorageResult<()> {
        debug!(user_id = %user_id, room_id = %room_id, "Forgetting room");

        // Remove the graph edge entirely (only if membership is 'leave')
        self.db()
            .query("DELETE member_of WHERE in = $user_rid AND out = $room_rid AND membership = 'leave'")
            .bind(("user_rid", user_rid_from_matrix_id(user_id)))
            .bind(("room_rid", room_rid(room_id)))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn store_room_upgrade(
        &self,
        old_room_id: &str,
        new_room_id: &str,
        version: &str,
        creator: &str,
        tombstone_event_id: &str,
    ) -> StorageResult<()> {
        debug!(old = %old_room_id, new = %new_room_id, version = %version, "Storing room upgrade edge");

        let old_rid = room_rid(old_room_id);
        let new_rid = room_rid(new_room_id);

        self.db()
            .query(
                "RELATE $old->upgrades_to->$new SET \
                 version = $ver, creator = $creator, tombstone_event_id = $tombstone",
            )
            .bind(("old", old_rid))
            .bind(("new", new_rid))
            .bind(("ver", version.to_string()))
            .bind(("creator", creator.to_string()))
            .bind(("tombstone", tombstone_event_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_room_predecessors(&self, room_id: &str) -> StorageResult<Vec<String>> {
        // Traverse the upgrade chain backward: find all rooms that upgraded TO this one,
        // then recursively find their predecessors.
        // SurrealDB graph traversal: room <-upgrades_to<- room <-upgrades_to<- room ...
        let mut predecessors = Vec::new();
        let mut current = room_id.to_string();

        // Iterative backward traversal (max depth 100 to prevent infinite loops)
        for _ in 0..100 {
            let mut response = self
                .db()
                .query(
                    "SELECT in.room_id AS room_id FROM upgrades_to WHERE out = $room_rid LIMIT 1",
                )
                .bind(("room_rid", room_rid(&current)))
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;

            let rows: Vec<RoomIdRow> = response
                .take(0)
                .map_err(|e| StorageError::Query(e.to_string()))?;

            match rows.into_iter().next() {
                Some(row) => {
                    predecessors.push(row.room_id.clone());
                    current = row.room_id;
                }
                None => break,
            }
        }

        Ok(predecessors)
    }
}
