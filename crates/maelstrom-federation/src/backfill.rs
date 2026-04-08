use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::error::MatrixError;

use crate::FederationState;

pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/federation/v1/backfill/{roomId}", get(backfill))
        .route(
            "/_matrix/federation/v1/get_missing_events/{roomId}",
            post(get_missing_events),
        )
}

#[derive(Deserialize)]
struct BackfillQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    /// Event ID to start backfilling from.
    v: Option<String>,
}

fn default_limit() -> usize {
    100
}

/// GET /_matrix/federation/v1/backfill/{roomId} — return historical events.
async fn backfill(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Query(query): Query<BackfillQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %room_id, limit = query.limit, "Backfill request");

    let limit = query.limit.min(500);

    // Get events backward from a starting point
    let from_pos = if let Some(ref event_id) = query.v {
        let event = state
            .storage()
            .get_event(event_id)
            .await
            .map_err(|_| MatrixError::not_found("Start event not found"))?;
        event.stream_position
    } else {
        // Start from the latest
        state
            .storage()
            .current_stream_position()
            .await
            .unwrap_or(i64::MAX)
    };

    let events = state
        .storage()
        .get_room_events(&room_id, from_pos, limit, "b")
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let pdus: Vec<serde_json::Value> = events.iter().map(|e| e.to_federation_event()).collect();

    Ok(Json(serde_json::json!({
        "origin": state.server_name().as_str(),
        "origin_server_ts": maelstrom_core::events::pdu::timestamp_ms(),
        "pdus": pdus,
    })))
}

#[derive(Deserialize)]
struct GetMissingEventsRequest {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    earliest_events: Vec<String>,
    #[serde(default)]
    latest_events: Vec<String>,
}

/// POST /_matrix/federation/v1/get_missing_events/{roomId}
async fn get_missing_events(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Json(body): Json<GetMissingEventsRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(
        room_id = %room_id,
        earliest = ?body.earliest_events,
        latest = ?body.latest_events,
        limit = body.limit,
        "get_missing_events request"
    );

    let limit = body.limit.min(500);

    // Simplified: return recent events in the room up to the limit.
    // Full implementation would walk the DAG between earliest and latest.
    let pos = state
        .storage()
        .current_stream_position()
        .await
        .unwrap_or(i64::MAX);

    let events = state
        .storage()
        .get_room_events(&room_id, pos, limit, "b")
        .await
        .unwrap_or_default();

    let pdus: Vec<serde_json::Value> = events.iter().map(|e| e.to_federation_event()).collect();

    Ok(Json(serde_json::json!({ "events": pdus })))
}
