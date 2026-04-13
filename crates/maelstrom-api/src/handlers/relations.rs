//! Event relations -- threads, reactions, edits, and aggregations.
//!
//! Events can relate to other events using the `m.relates_to` field. The server
//! tracks these relationships and serves them back through the relations
//! endpoints, both as raw child events and as server-side aggregations.
//!
//! The main relation types are:
//!
//! * **`m.annotation`** -- reactions (e.g. emoji). Aggregated into a count per
//!   key on the parent event.
//! * **`m.replace`** -- edits. The server replaces the parent event's content
//!   with the latest edit in aggregated views.
//! * **`m.thread`** -- threads. Groups reply chains under a root event,
//!   surfaced as a threaded conversation.
//! * **`m.reference`** -- generic references (e.g. verification events).
//!
//! The endpoints support filtering by relation type and/or event type, and
//! return paginated results.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/_matrix/client/v1/rooms/{roomId}/relations/{eventId}` | Get all relations for an event |
//! | `GET` | `/_matrix/client/v1/rooms/{roomId}/relations/{eventId}/{relType}` | Get relations filtered by relation type |
//! | `GET` | `/_matrix/client/v1/rooms/{roomId}/relations/{eventId}/{relType}/{eventType}` | Get relations filtered by both relation type and event type |
//!
//! # Matrix spec
//!
//! * [Relationships between events](https://spec.matrix.org/v1.18/client-server-api/#relationships-between-events)
//! * [Aggregations of child events](https://spec.matrix.org/v1.18/client-server-api/#aggregations-of-child-events)

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;

use crate::extractors::AuthenticatedUser;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/_matrix/client/v1/rooms/{roomId}/relations/{eventId}",
            get(get_relations),
        )
        .route(
            "/_matrix/client/v1/rooms/{roomId}/relations/{eventId}/{relType}",
            get(get_relations_by_type),
        )
        .route(
            "/_matrix/client/v1/rooms/{roomId}/relations/{eventId}/{relType}/{eventType}",
            get(get_relations_by_type_and_event_type),
        )
}

#[derive(Deserialize)]
struct RelationsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    from: Option<String>,
    dir: Option<String>,
}

fn default_limit() -> usize {
    50
}

#[derive(Deserialize)]
struct RelationsParams {
    #[allow(dead_code)]
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

#[derive(Deserialize)]
struct RelationsByTypeParams {
    #[allow(dead_code)]
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
    #[serde(rename = "relType")]
    rel_type: String,
}

#[derive(Deserialize)]
struct RelationsByTypeAndEventParams {
    #[allow(dead_code)]
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
    #[serde(rename = "relType")]
    rel_type: String,
    #[serde(rename = "eventType")]
    event_type: String,
}

/// GET /rooms/{roomId}/relations/{eventId} — all relations.
async fn get_relations(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<RelationsParams>,
    Query(query): Query<RelationsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    fetch_relations(
        &state,
        &params.event_id,
        None,
        None,
        query.limit,
        query.from.as_deref(),
        query.dir.as_deref(),
    )
    .await
}

/// GET /rooms/{roomId}/relations/{eventId}/{relType}
async fn get_relations_by_type(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<RelationsByTypeParams>,
    Query(query): Query<RelationsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    fetch_relations(
        &state,
        &params.event_id,
        Some(&params.rel_type),
        None,
        query.limit,
        query.from.as_deref(),
        query.dir.as_deref(),
    )
    .await
}

/// GET /rooms/{roomId}/relations/{eventId}/{relType}/{eventType}
async fn get_relations_by_type_and_event_type(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<RelationsByTypeAndEventParams>,
    Query(query): Query<RelationsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    fetch_relations(
        &state,
        &params.event_id,
        Some(&params.rel_type),
        Some(&params.event_type),
        query.limit,
        query.from.as_deref(),
        query.dir.as_deref(),
    )
    .await
}

async fn fetch_relations(
    state: &AppState,
    event_id: &str,
    rel_type: Option<&str>,
    event_type: Option<&str>,
    limit: usize,
    from: Option<&str>,
    dir: Option<&str>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let effective_limit = limit.min(100);
    let dir = dir.unwrap_or("b"); // default: newest first (backward)
    let from_pos: Option<i64> = from.and_then(|s| s.parse().ok());

    // Fetch all relations (no cursor at storage level — we paginate in handler)
    let relations = state
        .storage()
        .get_relations(event_id, rel_type, event_type, 1000, None)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Fetch the actual events and their stream positions
    let mut events_with_pos: Vec<(serde_json::Value, i64)> = Vec::new();
    for rel in &relations {
        if let Ok(event) = state.storage().get_event(&rel.event_id).await {
            let pos = event.stream_position;
            let json = event.to_client_event().into_json();
            events_with_pos.push((json, pos));
        }
    }

    // Sort by stream_position according to direction
    if dir == "f" {
        events_with_pos.sort_by_key(|(_, pos)| *pos);
    } else {
        events_with_pos.sort_by_key(|(_, pos)| std::cmp::Reverse(*pos));
    }

    // Apply from cursor filter (stream position based), then take limit
    let events_with_pos: Vec<_> = if let Some(cursor) = from_pos {
        events_with_pos
            .into_iter()
            .filter(|(_, pos)| {
                if dir == "f" {
                    *pos > cursor
                } else {
                    *pos < cursor
                }
            })
            .take(effective_limit)
            .collect()
    } else {
        events_with_pos.into_iter().take(effective_limit).collect()
    };

    let chunk: Vec<serde_json::Value> = events_with_pos.iter().map(|(e, _)| e.clone()).collect();

    // Build response with pagination tokens
    let mut response = serde_json::json!({ "chunk": chunk });
    if chunk.len() == effective_limit
        && let Some(last) = events_with_pos.last()
    {
        response["next_batch"] = serde_json::json!(last.1.to_string());
    }
    // Include prev_batch when we have a from token
    if from_pos.is_some()
        && let Some(first) = events_with_pos.first()
    {
        response["prev_batch"] = serde_json::json!(first.1.to_string());
    }

    Ok(Json(response))
}

/// Build bundled aggregations for an event (reactions, edits, thread summary).
/// Called when returning events to clients (sync, messages, etc).
pub async fn build_aggregations(
    storage: &dyn maelstrom_storage::traits::Storage,
    event_id: &str,
) -> Option<serde_json::Value> {
    let mut aggregations = serde_json::Map::new();

    // Reaction aggregations
    if let Ok(counts) = storage.get_reaction_counts(event_id).await
        && !counts.is_empty()
    {
        let chunk: Vec<serde_json::Value> = counts
            .iter()
            .map(|(key, count)| {
                serde_json::json!({
                    "type": "m.reaction",
                    "key": key,
                    "count": count,
                })
            })
            .collect();
        aggregations.insert(
            "m.annotation".to_string(),
            serde_json::json!({ "chunk": chunk }),
        );
    }

    // Latest edit
    if let Ok(Some(edit_event_id)) = storage.get_latest_edit(event_id).await
        && let Ok(edit_event) = storage.get_event(&edit_event_id).await
    {
        aggregations.insert(
            "m.replace".to_string(),
            edit_event.to_client_event().into_json(),
        );
    }

    // Thread summary
    if let Ok(thread_relations) = storage
        .get_relations(event_id, Some("m.thread"), None, 1, None)
        .await
        && !thread_relations.is_empty()
    {
        // Count total thread replies
        let all_replies = storage
            .get_relations(event_id, Some("m.thread"), None, 10000, None)
            .await
            .unwrap_or_default();

        let count = all_replies.len();

        // Find the reply with the highest stream_position (most recent)
        let mut latest_event_opt = None;
        let mut max_pos = i64::MIN;
        for rel in &all_replies {
            if let Ok(ev) = storage.get_event(&rel.event_id).await
                && ev.stream_position > max_pos
            {
                max_pos = ev.stream_position;
                latest_event_opt = Some(ev);
            }
        }

        if let Some(latest_event) = latest_event_opt {
            aggregations.insert(
                "m.thread".to_string(),
                serde_json::json!({
                    "latest_event": latest_event.to_client_event().into_json(),
                    "count": count,
                    "current_user_participated": false,
                }),
            );
        }
    }

    if aggregations.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "m.relations": aggregations }))
    }
}
