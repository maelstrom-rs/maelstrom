use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;

use crate::extractors::AuthenticatedUser;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v1/rooms/{roomId}/threads",
        get(get_threads),
    )
}

#[derive(Deserialize)]
struct ThreadsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    from: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    include: Option<String>,
}

fn default_limit() -> usize {
    50
}

/// GET /rooms/{roomId}/threads — list threads in a room.
async fn get_threads(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    Query(query): Query<ThreadsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let limit = query.limit.min(100);
    let from_pos = query.from.as_deref().and_then(|f| f.parse::<i64>().ok());

    // Get thread root event IDs
    let thread_roots = state
        .storage()
        .get_thread_roots(&room_id, limit, from_pos)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Fetch the actual root events with thread summaries
    let mut chunk = Vec::new();
    for root_id in &thread_roots {
        if let Ok(event) = state.storage().get_event(root_id).await {
            let mut client_event = event.to_client_event();

            // Add thread aggregation to unsigned
            if let Some(agg) = super::relations::build_aggregations(state.storage(), root_id).await
                && let Some(obj) = client_event.as_object_mut()
            {
                let unsigned = obj.entry("unsigned").or_insert(serde_json::json!({}));
                if let Some(u) = unsigned.as_object_mut() {
                    u.insert("m.relations".to_string(), agg["m.relations"].clone());
                }
            }

            chunk.push(client_event);
        }
    }

    let next_batch = if chunk.len() == limit {
        // Use the stream position of the last event as pagination token
        chunk
            .last()
            .and_then(|e| e.get("origin_server_ts"))
            .map(|ts| ts.to_string())
    } else {
        None
    };

    let mut response = serde_json::json!({ "chunk": chunk });
    if let Some(next) = next_batch {
        response["next_batch"] = serde_json::json!(next);
    }

    Ok(Json(response))
}
