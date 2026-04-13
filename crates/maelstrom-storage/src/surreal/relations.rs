//! Event relation storage -- [`RelationStore`](crate::traits::RelationStore) implementation.
//!
//! Relations are modeled as SurrealDB graph edges:
//! `event ->relates_to-> event` with metadata fields (`rel_type`, `sender`,
//! `event_type`, `content_key`).  This enables queries like "all reactions
//! on event X" or "latest edit of event Y" via graph traversal.
//!
//! Aggregated reaction counts use a `GROUP BY content_key` query with
//! `count()` to avoid scanning all child events on every read.
//!
//! Thread root discovery (`get_thread_roots`) finds parent events in a room
//! that have at least one `m.thread` child, ordered by stream position for
//! pagination.

use async_trait::async_trait;
use surrealdb::types::{RecordId, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Row from a relates_to query that joins with the event table to get event_id.
#[allow(dead_code)]
#[derive(Debug, Clone, SurrealValue)]
struct RelationQueryRow {
    rel_type: String,
    room_id: String,
    sender: String,
    event_type: String,
    content_key: Option<String>,
    child_event_id: String,
}

#[derive(Debug, Clone, SurrealValue)]
struct ReactionCountRow {
    content_key: String,
    count: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, SurrealValue)]
struct EventIdRow {
    child_event_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, SurrealValue)]
struct ParentEventIdRow {
    parent_event_id: String,
}

#[async_trait]
impl RelationStore for SurrealStorage {
    async fn store_relation(&self, relation: &RelationRecord) -> StorageResult<()> {
        debug!(
            event_id = %relation.event_id,
            parent_id = %relation.parent_id,
            rel_type = %relation.rel_type,
            "Storing graph relation"
        );

        let event_rid = RecordId::new("event", relation.event_id.as_str());
        let parent_rid = RecordId::new("event", relation.parent_id.as_str());

        // Create graph edge: child_event --relates_to--> parent_event
        // Store event IDs as plain strings on the edge for easy querying
        self.db()
            .query(
                "RELATE $from->relates_to->$to SET \
                 rel_type = $rtype, room_id = $rid, sender = $sender, \
                 event_type = $etype, content_key = $ckey, \
                 child_event_id = $child_eid, parent_event_id = $parent_eid",
            )
            .bind(("from", event_rid))
            .bind(("to", parent_rid))
            .bind(("child_eid", relation.event_id.clone()))
            .bind(("parent_eid", relation.parent_id.clone()))
            .bind(("rtype", relation.rel_type.clone()))
            .bind(("rid", relation.room_id.clone()))
            .bind(("sender", relation.sender.clone()))
            .bind(("etype", relation.event_type.clone()))
            .bind(("ckey", relation.content_key.clone()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_relations(
        &self,
        parent_id: &str,
        rel_type: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
        from: Option<&str>,
    ) -> StorageResult<Vec<RelationRecord>> {
        let parent_rid = RecordId::new("event", parent_id);

        // Query graph edges pointing to this parent.
        let mut query = String::from(
            "SELECT *, child_event_id \
             FROM relates_to WHERE out = $parent",
        );
        if rel_type.is_some() {
            query.push_str(" AND rel_type = $rtype");
        }
        if event_type.is_some() {
            query.push_str(" AND event_type = $etype");
        }
        if from.is_some() {
            query.push_str(" AND id < $from_id");
        }
        query.push_str(" ORDER BY id DESC LIMIT $lim");

        let mut response = self
            .db()
            .query(&query)
            .bind(("parent", parent_rid))
            .bind(("rtype", rel_type.unwrap_or("").to_string()))
            .bind(("etype", event_type.unwrap_or("").to_string()))
            .bind(("from_id", from.unwrap_or("").to_string()))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        // Use JSON deserialization for robustness
        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let child_eid = r
                    .get("child_event_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())?;
                Some(RelationRecord {
                    event_id: child_eid,
                    parent_id: parent_id.to_string(),
                    room_id: r
                        .get("room_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    rel_type: r
                        .get("rel_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    sender: r
                        .get("sender")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    event_type: r
                        .get("event_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    content_key: r
                        .get("content_key")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
            })
            .collect())
    }

    async fn get_reaction_counts(&self, parent_id: &str) -> StorageResult<Vec<(String, u64)>> {
        let parent_rid = RecordId::new("event", parent_id);

        let mut response = self
            .db()
            .query(
                "SELECT content_key, count() AS count FROM relates_to \
                 WHERE out = $parent AND rel_type = 'm.annotation' \
                 GROUP BY content_key",
            )
            .bind(("parent", parent_rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ReactionCountRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| (r.content_key, r.count as u64))
            .collect())
    }

    async fn get_latest_edit(&self, event_id: &str) -> StorageResult<Option<String>> {
        let event_rid = RecordId::new("event", event_id);

        let mut response = self
            .db()
            .query(
                "SELECT *, child_event_id FROM relates_to \
                 WHERE out = $parent AND rel_type = 'm.replace' \
                 ORDER BY id DESC LIMIT 1",
            )
            .bind(("parent", event_rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .next()
            .and_then(|r| r.get("child_event_id")?.as_str().map(|s| s.to_string())))
    }

    async fn get_thread_roots(
        &self,
        room_id: &str,
        limit: usize,
        _from: Option<i64>,
    ) -> StorageResult<Vec<String>> {
        // Fetch all thread relations with their child event IDs
        let mut response = self
            .db()
            .query(
                "SELECT parent_event_id, child_event_id \
                 FROM relates_to \
                 WHERE room_id = $rid AND rel_type = 'm.thread'",
            )
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<serde_json::Value> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        // Group by parent_event_id, track max child stream_position per group
        let mut thread_latest: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        for row in &rows {
            let parent = match row.get("parent_event_id").and_then(|v| v.as_str()) {
                Some(p) => p.to_string(),
                None => continue,
            };
            let child_id = match row.get("child_event_id").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => continue,
            };
            // Look up the child event's stream_position
            let pos = self
                .get_event(child_id)
                .await
                .map(|e| e.stream_position)
                .unwrap_or(0);
            let entry = thread_latest.entry(parent).or_insert(0);
            if pos > *entry {
                *entry = pos;
            }
        }

        // Sort by latest reply position descending
        let mut threads: Vec<(String, i64)> = thread_latest.into_iter().collect();
        threads.sort_by(|a, b| b.1.cmp(&a.1));
        threads.truncate(limit);

        Ok(threads.into_iter().map(|(root, _)| root).collect())
    }

    async fn store_report(&self, report: &ReportRecord) -> StorageResult<()> {
        self.db()
            .query(
                "CREATE event_report SET \
                 event_id = $eid, room_id = $rid, reporter = $rep, \
                 reason = $reason, score = $score",
            )
            .bind(("eid", report.event_id.clone()))
            .bind(("rid", report.room_id.clone()))
            .bind(("rep", report.reporter.clone()))
            .bind(("reason", report.reason.clone()))
            .bind(("score", report.score))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }
}
