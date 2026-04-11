//! # Room State and Event Queries
//!
//! This module provides federation endpoints for querying a room's state and
//! retrieving individual events. These are essential for servers that need to
//! verify room state or fetch events they are missing.
//!
//! ## Endpoints
//!
//! ### `GET /_matrix/federation/v1/state/{roomId}`
//!
//! Returns the **full current state** of a room as an array of PDUs, along with
//! the **auth chain** -- the set of events needed to verify those state events.
//! This is used by servers joining a room to bootstrap their view of the room's
//! current state (who is in the room, what the name is, permissions, etc.).
//!
//! ### `GET /_matrix/federation/v1/state_ids/{roomId}`
//!
//! A lightweight alternative that returns only the **event IDs** of the current
//! state and auth chain, not the full events. The requesting server can then
//! selectively fetch only the events it does not already have.
//!
//! ### `GET /_matrix/federation/v1/event/{eventId}`
//!
//! Returns a single event by its ID. Used when a server needs a specific event
//! it does not have -- for example, an event referenced in `auth_events` or
//! `prev_events` that was never received in a transaction.

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;

use crate::FederationState;
use crate::joins::compute_auth_chain;

/// Build the state query sub-router with state, state_ids, and event endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/federation/v1/state/{roomId}", get(get_room_state))
        .route(
            "/_matrix/federation/v1/state_ids/{roomId}",
            get(get_room_state_ids),
        )
        .route("/_matrix/federation/v1/event/{eventId}", get(get_event))
}

/// Query parameters for state endpoints.
///
/// The `event_id` parameter allows querying state at a specific point in the
/// room's history (not yet implemented -- currently returns current state).
#[derive(Deserialize)]
struct StateQuery {
    event_id: Option<String>,
}

/// GET /_matrix/federation/v1/state/{roomId} — return room state.
///
/// If the `event_id` query parameter is provided, returns the state at the point
/// of that event (all state events with stream_position <= the target event's
/// position). Otherwise returns the current state.
async fn get_room_state(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Query(query): Query<StateQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %room_id, event_id = ?query.event_id, "Federation state request");

    let state_events = if let Some(ref event_id) = query.event_id {
        // Get state at the point of this event
        let target = state
            .storage()
            .get_event(event_id)
            .await
            .map_err(|_| MatrixError::not_found("Event not found"))?;
        let all_state = state
            .storage()
            .get_current_state(&room_id)
            .await
            .unwrap_or_default();
        // Filter to events at or before the target position
        all_state
            .into_iter()
            .filter(|e| e.stream_position <= target.stream_position)
            .collect::<Vec<_>>()
    } else {
        state
            .storage()
            .get_current_state(&room_id)
            .await
            .map_err(|_| MatrixError::not_found("Room not found"))?
    };

    let pdus: Vec<serde_json::Value> = state_events
        .iter()
        .map(|e| e.to_federation_json())
        .collect();

    // Compute the proper auth chain: transitive closure of auth_events
    let auth_chain = compute_auth_chain(state.storage(), &state_events).await;

    Ok(Json(serde_json::json!({
        "pdus": pdus,
        "auth_chain": auth_chain,
    })))
}

/// GET /_matrix/federation/v1/state_ids/{roomId} — return event IDs of room state.
///
/// If the `event_id` query parameter is provided, returns the state IDs at the
/// point of that event. Otherwise returns the current state IDs.
async fn get_room_state_ids(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Query(query): Query<StateQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let state_events = if let Some(ref event_id) = query.event_id {
        let target = state
            .storage()
            .get_event(event_id)
            .await
            .map_err(|_| MatrixError::not_found("Event not found"))?;
        let all_state = state
            .storage()
            .get_current_state(&room_id)
            .await
            .unwrap_or_default();
        all_state
            .into_iter()
            .filter(|e| e.stream_position <= target.stream_position)
            .collect::<Vec<_>>()
    } else {
        state
            .storage()
            .get_current_state(&room_id)
            .await
            .map_err(|_| MatrixError::not_found("Room not found"))?
    };

    let pdu_ids: Vec<String> = state_events.iter().map(|e| e.event_id.clone()).collect();

    // Compute proper auth chain and extract just the event IDs
    let auth_chain = compute_auth_chain(state.storage(), &state_events).await;
    let auth_chain_ids: Vec<String> = auth_chain
        .iter()
        .filter_map(|e| {
            e.get("event_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

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
        "pdus": [event.to_federation_json()],
    })))
}
