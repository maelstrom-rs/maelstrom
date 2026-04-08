use async_trait::async_trait;
use surrealdb::types::{RecordId, SurrealValue};
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Row from a relates_to query that joins with the event table to get event_id.
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

#[derive(Debug, Clone, SurrealValue)]
struct EventIdRow {
    child_event_id: String,
}

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
        self.db()
            .query(
                "RELATE $from->relates_to->$to SET \
                 rel_type = $rtype, room_id = $rid, sender = $sender, \
                 event_type = $etype, content_key = $ckey"
            )
            .bind(("from", event_rid))
            .bind(("to", parent_rid))
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
        _from: Option<&str>,
    ) -> StorageResult<Vec<RelationRecord>> {
        let parent_rid = RecordId::new("event", parent_id);

        // Query graph edges pointing to this parent, join with child event to get event_id
        let mut query = String::from(
            "SELECT rel_type, room_id, sender, event_type, content_key, \
             in.event_id AS child_event_id \
             FROM relates_to WHERE out = $parent"
        );
        if rel_type.is_some() {
            query.push_str(" AND rel_type = $rtype");
        }
        if event_type.is_some() {
            query.push_str(" AND event_type = $etype");
        }
        query.push_str(" ORDER BY created_at ASC LIMIT $lim");

        let mut response = self
            .db()
            .query(&query)
            .bind(("parent", parent_rid))
            .bind(("rtype", rel_type.unwrap_or("").to_string()))
            .bind(("etype", event_type.unwrap_or("").to_string()))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<RelationQueryRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| RelationRecord {
                event_id: r.child_event_id,
                parent_id: parent_id.to_string(),
                room_id: r.room_id,
                rel_type: r.rel_type,
                sender: r.sender,
                event_type: r.event_type,
                content_key: r.content_key,
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

        Ok(rows.into_iter().map(|r| (r.content_key, r.count as u64)).collect())
    }

    async fn get_latest_edit(&self, event_id: &str) -> StorageResult<Option<String>> {
        let event_rid = RecordId::new("event", event_id);

        let mut response = self
            .db()
            .query(
                "SELECT in.event_id AS child_event_id FROM relates_to \
                 WHERE out = $parent AND rel_type = 'm.replace' \
                 ORDER BY created_at DESC LIMIT 1",
            )
            .bind(("parent", event_rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<EventIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().next().map(|r| r.child_event_id))
    }

    async fn get_thread_roots(&self, room_id: &str, limit: usize, _from: Option<i64>) -> StorageResult<Vec<String>> {
        // Find distinct parent event_ids that have m.thread children in this room
        let mut response = self
            .db()
            .query(
                "SELECT out.event_id AS parent_event_id FROM relates_to \
                 WHERE room_id = $rid AND rel_type = 'm.thread' \
                 GROUP BY out \
                 ORDER BY created_at DESC LIMIT $lim",
            )
            .bind(("rid", room_id.to_string()))
            .bind(("lim", limit as i64))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ParentEventIdRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|r| r.parent_event_id).collect())
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
