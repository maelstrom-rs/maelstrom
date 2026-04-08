use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::error::MatrixError;
use maelstrom_core::events::pdu::timestamp_ms;

use crate::FederationState;

pub fn routes() -> Router<FederationState> {
    Router::new()
        .route(
            "/_matrix/federation/v1/make_join/{roomId}/{userId}",
            get(make_join),
        )
        .route(
            "/_matrix/federation/v2/send_join/{roomId}/{eventId}",
            put(send_join),
        )
        .route(
            "/_matrix/federation/v1/make_leave/{roomId}/{userId}",
            get(make_leave),
        )
        .route(
            "/_matrix/federation/v2/send_leave/{roomId}/{eventId}",
            put(send_leave),
        )
}

#[derive(Deserialize)]
struct MakeJoinParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "userId")]
    user_id: String,
}

/// GET /_matrix/federation/v1/make_join/{roomId}/{userId}
/// Returns a join event template for the remote server to sign.
async fn make_join(
    State(state): State<FederationState>,
    Path(params): Path<MakeJoinParams>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, user_id = %params.user_id, "make_join request");

    // Verify room exists
    state
        .storage()
        .get_room(&params.room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found on this server"))?;

    let room = state
        .storage()
        .get_room(&params.room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    // Get auth events for the join (create, join_rules, power_levels, current member state)
    let auth_event_ids = get_auth_event_ids(state.storage(), &params.room_id).await;

    // Get forward extremities for prev_events
    let prev_events = get_latest_event_ids(state.storage(), &params.room_id).await;

    let event_template = serde_json::json!({
        "room_id": params.room_id,
        "sender": params.user_id,
        "type": "m.room.member",
        "state_key": params.user_id,
        "content": {
            "membership": "join",
        },
        "origin": params.user_id.split(':').nth(1).unwrap_or(""),
        "origin_server_ts": timestamp_ms(),
        "auth_events": auth_event_ids,
        "prev_events": prev_events,
        "depth": 100, // Simplified; real depth would be max(prev_events depths) + 1
    });

    Ok(Json(serde_json::json!({
        "event": event_template,
        "room_version": room.version,
    })))
}

#[derive(Deserialize)]
struct SendJoinParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

/// PUT /_matrix/federation/v2/send_join/{roomId}/{eventId}
/// Accept a signed join event from a remote server.
async fn send_join(
    State(state): State<FederationState>,
    Path(params): Path<SendJoinParams>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "send_join request");

    let sender = event_json
        .get("sender")
        .and_then(|s| s.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing sender"))?
        .to_string();

    let event_type = event_json
        .get("type")
        .and_then(|s| s.as_str())
        .unwrap_or("m.room.member");

    // Store the join event
    let stored = maelstrom_core::events::pdu::StoredEvent {
        event_id: params.event_id.clone(),
        room_id: params.room_id.clone(),
        sender: sender.clone(),
        event_type: event_type.to_string(),
        state_key: event_json.get("state_key").and_then(|s| s.as_str()).map(|s| s.to_string()),
        content: event_json.get("content").cloned().unwrap_or(serde_json::json!({})),
        origin_server_ts: event_json.get("origin_server_ts").and_then(|t| t.as_u64()).unwrap_or(0),
        unsigned: None,
        stream_position: 0,
        origin: event_json.get("origin").and_then(|s| s.as_str()).map(|s| s.to_string()),
        auth_events: event_json.get("auth_events").and_then(|a| {
            a.as_array().map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        }),
        prev_events: event_json.get("prev_events").and_then(|a| {
            a.as_array().map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        }),
        depth: event_json.get("depth").and_then(|d| d.as_i64()),
        hashes: event_json.get("hashes").cloned(),
        signatures: event_json.get("signatures").cloned(),
    };

    state
        .storage()
        .store_event(&stored)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to store join event");
            MatrixError::unknown("Failed to store event")
        })?;

    // Update membership
    state
        .storage()
        .set_membership(&sender, &params.room_id, "join")
        .await
        .map_err(|_| MatrixError::unknown("Failed to update membership"))?;

    // Update room state
    if let Some(state_key) = &stored.state_key {
        let _ = state
            .storage()
            .set_room_state(&params.room_id, event_type, state_key, &params.event_id)
            .await;
    }

    // Return current room state + auth chain
    let current_state = state
        .storage()
        .get_current_state(&params.room_id)
        .await
        .unwrap_or_default();

    let state_events: Vec<serde_json::Value> = current_state
        .iter()
        .map(|e| e.to_federation_event())
        .collect();

    // For alpha, auth_chain is the same as state (simplified)
    let auth_chain = state_events.clone();

    Ok(Json(serde_json::json!({
        "origin": state.server_name().as_str(),
        "state": state_events,
        "auth_chain": auth_chain,
        "event": event_json,
    })))
}

#[derive(Deserialize)]
struct MakeLeaveParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "userId")]
    user_id: String,
}

/// GET /_matrix/federation/v1/make_leave/{roomId}/{userId}
async fn make_leave(
    State(state): State<FederationState>,
    Path(params): Path<MakeLeaveParams>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, user_id = %params.user_id, "make_leave request");

    let room = state
        .storage()
        .get_room(&params.room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let auth_event_ids = get_auth_event_ids(state.storage(), &params.room_id).await;
    let prev_events = get_latest_event_ids(state.storage(), &params.room_id).await;

    let event_template = serde_json::json!({
        "room_id": params.room_id,
        "sender": params.user_id,
        "type": "m.room.member",
        "state_key": params.user_id,
        "content": {
            "membership": "leave",
        },
        "origin": params.user_id.split(':').nth(1).unwrap_or(""),
        "origin_server_ts": timestamp_ms(),
        "auth_events": auth_event_ids,
        "prev_events": prev_events,
        "depth": 100,
    });

    Ok(Json(serde_json::json!({
        "event": event_template,
        "room_version": room.version,
    })))
}

#[derive(Deserialize)]
struct SendLeaveParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

/// PUT /_matrix/federation/v2/send_leave/{roomId}/{eventId}
async fn send_leave(
    State(state): State<FederationState>,
    Path(params): Path<SendLeaveParams>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "send_leave request");

    let sender = event_json
        .get("sender")
        .and_then(|s| s.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing sender"))?
        .to_string();

    // Store the leave event
    let stored = maelstrom_core::events::pdu::StoredEvent {
        event_id: params.event_id.clone(),
        room_id: params.room_id.clone(),
        sender: sender.clone(),
        event_type: "m.room.member".to_string(),
        state_key: Some(sender.clone()),
        content: serde_json::json!({ "membership": "leave" }),
        origin_server_ts: event_json.get("origin_server_ts").and_then(|t| t.as_u64()).unwrap_or(0),
        unsigned: None,
        stream_position: 0,
        origin: event_json.get("origin").and_then(|s| s.as_str()).map(|s| s.to_string()),
        auth_events: None,
        prev_events: None,
        depth: event_json.get("depth").and_then(|d| d.as_i64()),
        hashes: event_json.get("hashes").cloned(),
        signatures: event_json.get("signatures").cloned(),
    };

    let _ = state.storage().store_event(&stored).await;

    // Update membership
    let _ = state
        .storage()
        .set_membership(&sender, &params.room_id, "leave")
        .await;

    // Update room state
    let _ = state
        .storage()
        .set_room_state(&params.room_id, "m.room.member", &sender, &params.event_id)
        .await;

    Ok(Json(serde_json::json!({})))
}

/// Get auth event IDs for a room (create, join_rules, power_levels events).
async fn get_auth_event_ids(storage: &dyn maelstrom_storage::traits::Storage, room_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (event_type, state_key) in [
        ("m.room.create", ""),
        ("m.room.join_rules", ""),
        ("m.room.power_levels", ""),
    ] {
        if let Ok(event) = storage.get_state_event(room_id, event_type, state_key).await {
            ids.push(event.event_id);
        }
    }
    ids
}

/// Get the latest event IDs in a room (simplified: last 2 events by stream position).
async fn get_latest_event_ids(storage: &dyn maelstrom_storage::traits::Storage, room_id: &str) -> Vec<String> {
    if let Ok(pos) = storage.current_stream_position().await
        && let Ok(events) = storage.get_room_events(room_id, pos + 1, 2, "b").await {
            return events.into_iter().map(|e| e.event_id).collect();
        }
    Vec::new()
}
