//! # Federation Invite Flow
//!
//! When a user on server A invites a user on server B to a room, the invite must
//! be delivered over federation. This module handles the **receiving** side -- when
//! a remote server sends us an invite for one of our local users.
//!
//! ## How Federation Invites Work
//!
//! 1. The inviting server constructs an `m.room.member` event with `membership: invite`
//!    and the invited user's ID as the `state_key`.
//! 2. The inviting server sends this event to the invited user's server via
//!    `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}`.
//! 3. The receiving server (this code):
//!    - Verifies the invited user belongs to this server
//!    - Creates the room locally if it does not already exist (since the invited user
//!      may not have seen this room before)
//!    - Stores the invite event
//!    - Sets the user's membership to "invite" in that room
//!    - Returns the event (potentially with this server's signature added)
//!
//! ## V1 vs V2
//!
//! - **V2** (`PUT /v2/invite/{roomId}/{eventId}`): the event is wrapped in
//!   `{"event": {...}}`. The response is `{"event": {...}}`.
//! - **V1** (`PUT /v1/invite/{roomId}/{eventId}`): the event IS the request body
//!   (unwrapped). The response is `[200, {...}]` (array with status code).
//!
//! ## Endpoints
//!
//! - `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}` -- receive invite (v2 format)
//! - `PUT /_matrix/federation/v1/invite/{roomId}/{eventId}` -- receive invite (v1 format)

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::Pdu;
use maelstrom_core::matrix::room::Membership;
use maelstrom_core::matrix::room::event_type as et;

use crate::FederationState;

/// Build the invite sub-router with v1 and v2 invite endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route(
            "/_matrix/federation/v2/invite/{roomId}/{eventId}",
            put(receive_invite_v2),
        )
        .route(
            "/_matrix/federation/v1/invite/{roomId}/{eventId}",
            put(receive_invite_v1),
        )
}

#[derive(Deserialize)]
struct InviteParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

/// PUT /_matrix/federation/v2/invite/{roomId}/{eventId}
/// Accept an invite from a remote server for a local user.
async fn receive_invite_v2(
    State(state): State<FederationState>,
    Path(params): Path<InviteParams>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "Received federation invite v2");

    let event_json = body.get("event").unwrap_or(&body);
    let invite_event = store_invite_event(&state, &params, event_json).await?;

    Ok(Json(serde_json::json!({
        "event": invite_event,
    })))
}

/// PUT /_matrix/federation/v1/invite/{roomId}/{eventId}
/// Accept an invite (v1 format — event is the body itself, not wrapped in {"event": ...}).
async fn receive_invite_v1(
    State(state): State<FederationState>,
    Path(params): Path<InviteParams>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<(http::StatusCode, Json<serde_json::Value>), MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "Received federation invite v1");

    let invite_event = store_invite_event(&state, &params, &event_json).await?;

    // v1 returns [200, {...}] (array with status code and event)
    Ok((
        http::StatusCode::OK,
        Json(serde_json::json!([200, invite_event])),
    ))
}

/// Shared logic for processing an inbound federation invite (used by both v1 and v2).
///
/// Validates the invited user belongs to this server, creates the room locally if
/// needed, stores the invite event, sets membership to "invite", and updates room state.
async fn store_invite_event(
    state: &FederationState,
    params: &InviteParams,
    event_json: &serde_json::Value,
) -> Result<serde_json::Value, MatrixError> {
    let sender = event_json
        .get("sender")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let state_key = event_json
        .get("state_key")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    // The state_key is the invited user
    if state_key.is_empty() {
        return Err(MatrixError::bad_json("Missing state_key (invited user)"));
    }

    // Verify the invited user belongs to our server
    let user_server = maelstrom_core::matrix::id::server_name_from_sigil_id(&state_key);
    if user_server != state.server_name().as_str() {
        return Err(MatrixError::forbidden("Invited user is not on this server"));
    }

    // Check server ACL for the sender's server (if room already exists locally)
    let sender_server = maelstrom_core::matrix::id::server_name_from_sigil_id(&sender);
    if !sender_server.is_empty() {
        check_server_acl(state.storage(), &params.room_id, sender_server).await?;
    }

    // Create room locally if it doesn't exist (we only know about it through the invite)
    let room_version = event_json
        .get("room_version")
        .and_then(|v| v.as_str())
        .unwrap_or("10")
        .to_string();

    let room_record = maelstrom_storage::traits::RoomRecord {
        room_id: params.room_id.clone(),
        version: room_version,
        creator: sender.clone(),
        is_direct: event_json
            .get("content")
            .and_then(|c| c.get("is_direct"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    let _ = state.storage().create_room(&room_record).await;

    // Store the invite event
    let stored = Pdu {
        event_id: params.event_id.clone(),
        room_id: params.room_id.clone(),
        sender,
        event_type: et::MEMBER.to_string(),
        state_key: Some(state_key.clone()),
        content: event_json
            .get("content")
            .cloned()
            .unwrap_or(serde_json::json!({"membership": Membership::Invite.as_str()})),
        origin_server_ts: event_json
            .get("origin_server_ts")
            .and_then(|t| t.as_u64())
            .unwrap_or(0),
        unsigned: event_json.get("unsigned").cloned(),
        stream_position: 0,
        origin: event_json
            .get("origin")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        auth_events: event_json.get("auth_events").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        prev_events: event_json.get("prev_events").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        depth: event_json.get("depth").and_then(|d| d.as_i64()),
        hashes: event_json.get("hashes").cloned(),
        signatures: event_json.get("signatures").cloned(),
    };

    let _ = state.storage().store_event(&stored).await;

    // Set membership to invite
    state
        .storage()
        .set_membership(&state_key, &params.room_id, Membership::Invite.as_str())
        .await
        .map_err(|_| MatrixError::unknown("Failed to set invite membership"))?;

    // Update room state
    let _ = state
        .storage()
        .set_room_state(&params.room_id, et::MEMBER, &state_key, &params.event_id)
        .await;

    // Return the event (potentially with our signature added)
    Ok(event_json.clone())
}

/// Check if a server is allowed by the room's `m.room.server_acl` state event.
async fn check_server_acl(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    server_name: &str,
) -> Result<(), MatrixError> {
    let acl_event = match storage.get_state_event(room_id, et::SERVER_ACL, "").await {
        Ok(event) => event,
        Err(_) => return Ok(()),
    };

    let content = &acl_event.content;
    let allow_ip_literals = content
        .get("allow_ip_literals")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let allow = content
        .get("allow")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let deny = content
        .get("deny")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    if !allow_ip_literals {
        let host = server_name.split(':').next().unwrap_or(server_name);
        let first_char = host.chars().next().unwrap_or(' ');
        if first_char.is_ascii_digit() || first_char == '[' {
            return Err(MatrixError::forbidden(
                "Server ACL denies IP literal server names",
            ));
        }
    }

    for pattern in &deny {
        if server_acl_glob_match(pattern, server_name) {
            return Err(MatrixError::forbidden("Server is denied by room ACL"));
        }
    }

    if allow.is_empty() {
        return Err(MatrixError::forbidden("Server ACL allows no servers"));
    }
    for pattern in &allow {
        if server_acl_glob_match(pattern, server_name) {
            return Ok(());
        }
    }

    Err(MatrixError::forbidden("Server not in room ACL allow list"))
}

fn server_acl_glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    pattern == value
}
