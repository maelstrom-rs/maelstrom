use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;

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
    fetch_relations(&state, &params.event_id, None, None, query.limit, query.from.as_deref()).await
}

/// GET /rooms/{roomId}/relations/{eventId}/{relType}
async fn get_relations_by_type(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<RelationsByTypeParams>,
    Query(query): Query<RelationsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    fetch_relations(&state, &params.event_id, Some(&params.rel_type), None, query.limit, query.from.as_deref()).await
}

/// GET /rooms/{roomId}/relations/{eventId}/{relType}/{eventType}
async fn get_relations_by_type_and_event_type(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<RelationsByTypeAndEventParams>,
    Query(query): Query<RelationsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    fetch_relations(&state, &params.event_id, Some(&params.rel_type), Some(&params.event_type), query.limit, query.from.as_deref()).await
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
            chunk.push(event.to_client_event());
        }
    }

    Ok(Json(serde_json::json!({
        "chunk": chunk,
    })))
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
        && !counts.is_empty() {
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
        && let Ok(edit_event) = storage.get_event(&edit_event_id).await {
            aggregations.insert(
                "m.replace".to_string(),
                edit_event.to_client_event(),
            );
        }

    // Thread summary
    if let Ok(thread_relations) = storage
        .get_relations(event_id, Some("m.thread"), None, 1, None)
        .await
        && !thread_relations.is_empty() {
            // Count total thread replies
            let all_replies = storage
                .get_relations(event_id, Some("m.thread"), None, 10000, None)
                .await
                .unwrap_or_default();

            let count = all_replies.len();
            let latest = all_replies.last();

            if let Some(latest_rel) = latest
                && let Ok(latest_event) = storage.get_event(&latest_rel.event_id).await {
                    aggregations.insert(
                        "m.thread".to_string(),
                        serde_json::json!({
                            "latest_event": latest_event.to_client_event(),
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
