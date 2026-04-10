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
//! * [Relationships between events](https://spec.matrix.org/v1.12/client-server-api/#relationships-between-events)
//! * [Aggregations of child events](https://spec.matrix.org/v1.12/client-server-api/#aggregations-of-child-events)

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
    #[allow(dead_code)]
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
) -> Result<Json<serde_json::Value>, MatrixError> {
    let relations = state
        .storage()
        .get_relations(event_id, rel_type, event_type, limit.min(100), from)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Fetch the actual events for each relation
    let mut chunk = Vec::new();
    for rel in &relations {
        if let Ok(event) = state.storage().get_event(&rel.event_id).await {
            chunk.push(event.to_client_event().into_json());
        }
    }

    // Return next_batch if there are more results (we fetched limit items)
    let mut response = serde_json::json!({ "chunk": chunk });
    if chunk.len() == limit.min(100) {
        // Use the last event_id as the cursor token
        if let Some(last) = relations.last() {
            response["next_batch"] = serde_json::json!(last.event_id);
        }
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
        let latest = all_replies.last();

        if let Some(latest_rel) = latest
            && let Ok(latest_event) = storage.get_event(&latest_rel.event_id).await
        {
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
