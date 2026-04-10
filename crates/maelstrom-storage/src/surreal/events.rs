//! Event storage operations -- [`EventStore`](crate::traits::EventStore) implementation.
//!
//! Events (PDUs) are stored in the `event` table, each assigned a monotonically
//! increasing `stream_position` that drives `/sync` pagination.
//!
//! The current room state map is maintained in a separate `room_state` table
//! keyed by `(room_id, event_type, state_key)`, pointing to the latest
//! `event_id` for that slot.
//!
//! Full-text search (`search_events`) uses SurrealDB's built-in full-text
//! index on `content.body` with the `@@ (match)` operator and
//! `search::score()` for BM25 relevance ranking.
//!
//! Transaction-ID deduplication (`store_txn_id` / `get_txn_event`) prevents
//! duplicate event creation when a client retries a request.

use async_trait::async_trait;
use maelstrom_core::matrix::event::Pdu;
use surrealdb::types::{RecordId, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Row returned when reading an event record.
#[derive(Debug, Clone, SurrealValue)]
struct EventRow {
    event_id: String,
    room_id: String,
    sender: String,
    event_type: String,
    state_key: Option<String>,
    content: serde_json::Value,
    origin_server_ts: i64,
    unsigned_data: Option<serde_json::Value>,
    stream_position: i64,
    // Federation fields
    origin: Option<String>,
    auth_events: Option<Vec<String>>,
    prev_events: Option<Vec<String>>,
    depth: Option<i64>,
    hashes: Option<serde_json::Value>,
    signatures: Option<serde_json::Value>,
}

impl EventRow {
    fn into_pdu(self) -> Pdu {
        Pdu {
            event_id: self.event_id,
            room_id: self.room_id,
            sender: self.sender,
            event_type: self.event_type,
            state_key: self.state_key,
            content: self.content,
            origin_server_ts: self.origin_server_ts as u64,
            unsigned: self.unsigned_data,
            stream_position: self.stream_position,
            origin: self.origin,
            auth_events: self.auth_events,
            prev_events: self.prev_events,
            depth: self.depth,
            hashes: self.hashes,
            signatures: self.signatures,
        }
    }
}

/// Row returned when reading stream_counter position.
#[derive(Debug, Clone, SurrealValue)]
struct PositionRow {
    position: i64,
}

/// Row returned when reading room_state entries.
#[derive(Debug, Clone, SurrealValue)]
struct RoomStateRow {
    event_id: String,
}

/// Row returned when reading txn_id entries.
#[derive(Debug, Clone, SurrealValue)]
struct TxnIdRow {
    event_id: String,
}

#[async_trait]
impl EventStore for SurrealStorage {
    async fn store_event(&self, event: &Pdu) -> StorageResult<i64> {
        debug!(event_id = %event.event_id, room_id = %event.room_id, "Storing event");

        // Get the next stream position with retry
        let pos = match self.next_stream_position().await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "stream_position failed, using timestamp fallback");
                // Fallback: use current timestamp in microseconds as position
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as i64
            }
        };

        let rid = RecordId::new("event", event.event_id.as_str());

        // Use INSERT with ON DUPLICATE KEY UPDATE for idempotency
        let mut response = self
            .db()
            .query(
                "INSERT INTO event { \
                 id: $rid, \
                 event_id: $event_id, \
                 room_id: $room_id, \
                 sender: $sender, \
                 event_type: $event_type, \
                 state_key: $state_key, \
                 content: $content, \
                 origin_server_ts: $origin_server_ts, \
                 unsigned_data: $unsigned_data, \
                 stream_position: $pos, \
                 origin: $origin, \
                 auth_events: $auth_events, \
                 prev_events: $prev_events, \
                 depth: $depth, \
                 hashes: $hashes, \
                 signatures: $signatures \
                 } ON DUPLICATE KEY UPDATE stream_position = $pos",
            )
            .bind(("rid", rid))
            .bind(("event_id", event.event_id.clone()))
            .bind(("room_id", event.room_id.clone()))
            .bind(("sender", event.sender.clone()))
            .bind(("event_type", event.event_type.clone()))
            .bind(("state_key", event.state_key.clone()))
            .bind(("content", event.content.clone()))
            .bind(("origin_server_ts", event.origin_server_ts as i64))
            .bind(("unsigned_data", event.unsigned.clone()))
            .bind(("pos", pos))
            .bind(("origin", event.origin.clone()))
            .bind(("auth_events", event.auth_events.clone()))
            .bind(("prev_events", event.prev_events.clone()))
            .bind(("depth", event.depth))
            .bind(("hashes", event.hashes.clone()))
            .bind(("signatures", event.signatures.clone()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let _: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        // Create DAG graph edges for prev_events and auth_events
        let event_rid = RecordId::new("event", event.event_id.as_str());
        if let Some(prev) = &event.prev_events {
            for prev_id in prev {
                let prev_rid = RecordId::new("event", prev_id.as_str());
                let _ = self
                    .db()
                    .query("RELATE $from->event_edge->$to SET edge_type = 'prev'")
                    .bind(("from", event_rid.clone()))
                    .bind(("to", prev_rid))
                    .await;
            }
        }
        if let Some(auth) = &event.auth_events {
            for auth_id in auth {
                let auth_rid = RecordId::new("event", auth_id.as_str());
                let _ = self
                    .db()
                    .query("RELATE $from->event_edge->$to SET edge_type = 'auth'")
                    .bind(("from", event_rid.clone()))
                    .bind(("to", auth_rid))
                    .await;
            }
        }

        Ok(pos)
    }

    async fn get_event(&self, event_id: &str) -> StorageResult<Pdu> {
        let rid = RecordId::new("event", event_id);

        let result: Option<EventRow> = self
            .db()
            .select(rid)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        result
            .map(|row| row.into_pdu())
            .ok_or(StorageError::NotFound)
    }

    async fn get_room_events(
        &self,
        room_id: &str,
        from: i64,
        limit: usize,
        dir: &str,
    ) -> StorageResult<Vec<Pdu>> {
        let query = if dir == "b" {
            "SELECT * FROM event WHERE room_id = $rid AND stream_position < $from \
             ORDER BY stream_position DESC LIMIT $lim"
        } else {
            "SELECT * FROM event WHERE room_id = $rid AND stream_position > $from \
             ORDER BY stream_position ASC LIMIT $lim"
        };

        let mut response = self
            .db()
            .query(query)
            .bind(("rid", room_id.to_string()))
            .bind(("from", from))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<EventRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_pdu()).collect())
    }

    async fn get_events_since(&self, since: i64) -> StorageResult<Vec<Pdu>> {
        let mut response = self
            .db()
            .query(
                "SELECT * FROM event WHERE stream_position > $since \
                 ORDER BY stream_position ASC",
            )
            .bind(("since", since))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<EventRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let events: Vec<Pdu> = rows.into_iter().map(|r| r.into_pdu()).collect();
        if !events.is_empty() {
            debug!(since = %since, count = %events.len(), "get_events_since returned events");
        }
        Ok(events)
    }

    async fn set_room_state(
        &self,
        room_id: &str,
        event_type: &str,
        state_key: &str,
        event_id: &str,
    ) -> StorageResult<()> {
        debug!(
            room_id = %room_id,
            event_type = %event_type,
            state_key = %state_key,
            event_id = %event_id,
            "Setting room state"
        );

        let rid = room_id.to_string();
        let etype = event_type.to_string();
        let skey = state_key.to_string();
        let eid = event_id.to_string();

        // Upsert using INSERT ... ON DUPLICATE KEY UPDATE
        self.db()
            .query(
                "INSERT INTO room_state { room_id: $rid, event_type: $etype, state_key: $skey, event_id: $eid } \
                 ON DUPLICATE KEY UPDATE event_id = $eid",
            )
            .bind(("rid", rid))
            .bind(("etype", etype))
            .bind(("skey", skey))
            .bind(("eid", eid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_current_state(&self, room_id: &str) -> StorageResult<Vec<Pdu>> {
        // First get all event_ids from room_state for this room.
        let mut response = self
            .db()
            .query("SELECT event_id FROM room_state WHERE room_id = $rid")
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let state_rows: Vec<RoomStateRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        if state_rows.is_empty() {
            return Ok(Vec::new());
        }

        // Batch-fetch all events in a single query instead of N+1.
        let event_ids: Vec<String> = state_rows.into_iter().map(|r| r.event_id).collect();

        let mut response = self
            .db()
            .query("SELECT * FROM event WHERE event_id IN $ids")
            .bind(("ids", event_ids))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<EventRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_pdu()).collect())
    }

    async fn get_state_event(
        &self,
        room_id: &str,
        event_type: &str,
        state_key: &str,
    ) -> StorageResult<Pdu> {
        let mut response = self
            .db()
            .query(
                "SELECT event_id FROM room_state \
                 WHERE room_id = $rid AND event_type = $etype AND state_key = $skey",
            )
            .bind(("rid", room_id.to_string()))
            .bind(("etype", event_type.to_string()))
            .bind(("skey", state_key.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RoomStateRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let event_id = rows
            .into_iter()
            .next()
            .map(|r| r.event_id)
            .ok_or(StorageError::NotFound)?;

        self.get_event(&event_id).await
    }

    async fn get_state_event_at(
        &self,
        room_id: &str,
        event_type: &str,
        state_key: &str,
        at_position: i64,
    ) -> StorageResult<Pdu> {
        // Find the most recent state event of this type that was stored at or before the given position
        let mut response = self
            .db()
            .query(
                "SELECT * FROM event \
                 WHERE room_id = $rid AND event_type = $etype AND state_key = $skey \
                 AND stream_position <= $pos \
                 ORDER BY stream_position DESC LIMIT 1",
            )
            .bind(("rid", room_id.to_string()))
            .bind(("etype", event_type.to_string()))
            .bind(("skey", state_key.to_string()))
            .bind(("pos", at_position))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<EventRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|r| r.into_pdu())
            .ok_or(StorageError::NotFound)
    }

    async fn next_stream_position(&self) -> StorageResult<i64> {
        let rid = RecordId::new("stream_counter", "global");

        // Atomically increment with retry for concurrent access.
        for attempt in 0..3 {
            let result = self
                .db()
                .query("UPDATE $rid SET position += 1 RETURN AFTER")
                .bind(("rid", rid.clone()))
                .await;

            match result {
                Ok(mut response) => {
                    let rows: Vec<PositionRow> = response
                        .take(0)
                        .map_err(|e| StorageError::Query(e.to_string()))?;

                    return rows.into_iter().next().map(|r| r.position).ok_or(
                        StorageError::Internal(
                            "stream_counter:global not found — schema bootstrap may have failed"
                                .to_string(),
                        ),
                    );
                }
                Err(e) if attempt < 2 => {
                    tracing::warn!(attempt = attempt, error = %e, "stream_position retry");
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    continue;
                }
                Err(e) => return Err(StorageError::Query(e.to_string())),
            }
        }
        Err(StorageError::Internal(
            "stream_position exhausted retries".to_string(),
        ))
    }

    async fn current_stream_position(&self) -> StorageResult<i64> {
        let rid = RecordId::new("stream_counter", "global");

        let result: Option<PositionRow> = self
            .db()
            .select(rid)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(result.map(|r| r.position).unwrap_or(0))
    }

    async fn store_txn_id(
        &self,
        device_id: &str,
        room_id: &str,
        txn_id: &str,
        event_id: &str,
    ) -> StorageResult<()> {
        debug!(device_id = %device_id, txn_id = %txn_id, event_id = %event_id, "Storing txn_id");

        // Use a deterministic record ID so duplicate stores are idempotent.
        // The first store wins — CREATE on existing record is silently ignored.
        let record_key = format!("{device_id}:{room_id}:{txn_id}");
        let rid = RecordId::new("txn_id", record_key.as_str());

        // INSERT with ON DUPLICATE KEY — first store wins (no-op update preserves original event_id)
        let _ = self
            .db()
            .query(
                "INSERT INTO txn_id { id: $rid, device_id: $did, room_id: $roomid, txn_id: $tid, event_id: $eid } \
                 ON DUPLICATE KEY UPDATE device_id = device_id",
            )
            .bind(("rid", rid))
            .bind(("did", device_id.to_string()))
            .bind(("roomid", room_id.to_string()))
            .bind(("tid", txn_id.to_string()))
            .bind(("eid", event_id.to_string()))
            .await;

        Ok(())
    }

    async fn get_txn_event(
        &self,
        device_id: &str,
        room_id: &str,
        txn_id: &str,
    ) -> StorageResult<Option<String>> {
        // Use the deterministic record ID for direct lookup
        let record_key = format!("{device_id}:{room_id}:{txn_id}");
        let rid = RecordId::new("txn_id", record_key.as_str());

        let mut response = self
            .db()
            .query("SELECT event_id FROM ONLY $rid")
            .bind(("rid", rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<TxnIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().next().map(|r| r.event_id))
    }

    async fn search_events(
        &self,
        room_ids: &[String],
        query: &str,
        limit: usize,
    ) -> StorageResult<Vec<Pdu>> {
        debug!(query = %query, rooms = ?room_ids, limit = %limit, "Searching events");

        let mut response = self
            .db()
            .query(
                "SELECT *, search::score(1) AS relevance \
                 FROM event \
                 WHERE content.body @1@ $query AND room_id IN $rooms \
                 ORDER BY relevance DESC \
                 LIMIT $lim",
            )
            .bind(("query", query.to_string()))
            .bind(("rooms", room_ids.to_vec()))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<EventRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.into_pdu()).collect())
    }

    async fn redact_event(&self, event_id: &str) -> StorageResult<()> {
        debug!(event_id = %event_id, "Redacting event");

        self.db()
            .query("UPDATE event SET content = {} WHERE event_id = $eid")
            .bind(("eid", event_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }
}
