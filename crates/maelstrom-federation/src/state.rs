use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::error::MatrixError;

use crate::FederationState;

pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/federation/v1/state/{roomId}", get(get_room_state))
        .route(
            "/_matrix/federation/v1/state_ids/{roomId}",
            get(get_room_state_ids),
        )
        .route("/_matrix/federation/v1/event/{eventId}", get(get_event))
}

#[derive(Deserialize)]
struct StateQuery {
    #[allow(dead_code)]
    event_id: Option<String>,
}

/// GET /_matrix/federation/v1/state/{roomId} — return current room state.
async fn get_room_state(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Query(_query): Query<StateQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %room_id, "Federation state request");

    let current_state = state
        .storage()
        .get_current_state(&room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let pdus: Vec<serde_json::Value> = current_state
        .iter()
        .map(|e| e.to_federation_event())
        .collect();

    // Simplified auth_chain = same as state for alpha
    let auth_chain = pdus.clone();

    Ok(Json(serde_json::json!({
        "pdus": pdus,
        "auth_chain": auth_chain,
    })))
}

/// GET /_matrix/federation/v1/state_ids/{roomId} — return event IDs of room state.
async fn get_room_state_ids(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Query(_query): Query<StateQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let current_state = state
        .storage()
        .get_current_state(&room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let pdu_ids: Vec<String> = current_state.iter().map(|e| e.event_id.clone()).collect();
    let auth_chain_ids = pdu_ids.clone();

    Ok(Json(serde_json::json!({
        "pdu_ids": pdu_ids,
        "auth_chain_ids": auth_chain_ids,
    })))
}

/// GET /_matrix/federation/v1/event/{eventId} — return a single event.
async fn get_event(
    State(state): State<FederationState>,
    Path(event_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let event = state
        .storage()
        .get_event(&event_id)
        .await
        .map_err(|_| MatrixError::not_found("Event not found"))?;

    Ok(Json(serde_json::json!({
        "origin": state.server_name().as_str(),
        "origin_server_ts": event.origin_server_ts,
        "pdus": [event.to_federation_event()],
    })))
}
