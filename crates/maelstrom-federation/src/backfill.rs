//! # Backfill and Missing Events
//!
//! Matrix rooms are built on a **DAG** (Directed Acyclic Graph) of events. When a
//! server joins a room or has gaps in its copy of the DAG, it needs to fetch
//! historical events from other servers. This module provides two mechanisms:
//!
//! ## Backfill
//!
//! `GET /_matrix/federation/v1/backfill/{roomId}` returns a batch of historical
//! events walking **backward** from a given starting event. This is used when a
//! client scrolls up in a room and the local server does not have older events.
//!
//! Query parameters:
//! - `v` -- the event ID to start backfilling from (defaults to the latest event)
//! - `limit` -- maximum number of events to return (capped at 500)
//!
//! ## Get Missing Events
//!
//! `POST /_matrix/federation/v1/get_missing_events/{roomId}` is used to fill gaps
//! in the event DAG. The requesting server provides:
//! - `earliest_events` -- event IDs it already has (the "known" frontier)
//! - `latest_events` -- event IDs it has seen referenced but does not have
//! - `limit` -- maximum events to return
//!
//! The responding server walks the DAG backward from `latest_events` toward
//! `earliest_events` and returns the events in between. The current implementation
//! is simplified -- it returns recent events up to the limit rather than performing
//! a full DAG walk.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;

use crate::FederationState;

/// Build the backfill sub-router with historical event retrieval endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/federation/v1/backfill/{roomId}", get(backfill))
        .route(
            "/_matrix/federation/v1/get_missing_events/{roomId}",
            post(get_missing_events),
        )
}

/// Query parameters for the backfill endpoint.
#[derive(Deserialize)]
struct BackfillQuery {
    /// Maximum number of events to return (default: 100, capped at 500).
    #[serde(default = "default_limit")]
    limit: usize,
    /// Event ID to start backfilling from. If omitted, starts from the latest event.
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

    let pdus: Vec<serde_json::Value> = events.iter().map(|e| e.to_federation_json()).collect();

    Ok(Json(serde_json::json!({
        "origin": state.server_name().as_str(),
        "origin_server_ts": maelstrom_core::matrix::event::timestamp_ms(),
        "pdus": pdus,
    })))
}

/// Request body for the get_missing_events endpoint.
#[derive(Deserialize)]
struct GetMissingEventsRequest {
    /// Maximum number of events to return (capped at 500).
    #[serde(default = "default_limit")]
    limit: usize,
    /// Event IDs the requesting server already has (the "known" frontier).
    #[serde(default)]
    earliest_events: Vec<String>,
    /// Event IDs the requesting server has seen referenced but does not possess.
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

    let pdus: Vec<serde_json::Value> = events.iter().map(|e| e.to_federation_json()).collect();

    Ok(Json(serde_json::json!({ "events": pdus })))
}
