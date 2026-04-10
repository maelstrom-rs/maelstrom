//! Room knocking.
//!
//! Knocking lets a user request access to a room they cannot join directly.
//! The flow is:
//!
//! 1. The user **knocks** on the room (this endpoint). A membership event with
//!    `membership: knock` is inserted into the room state.
//! 2. An existing member with sufficient power level sees the knock and decides
//!    to **invite** the user.
//! 3. The user **joins** the room using the invite.
//!
//! Knocking is only allowed when the room's join rules include `knock` (or
//! `knock_restricted`). If the user is already a member, banned, or the join
//! rules do not permit knocking, the request is rejected.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST` | `/_matrix/client/v3/knock/{roomIdOrAlias}` | Knock on a room to request an invite |
//!
//! # Matrix spec
//!
//! * [Knocking on rooms](https://spec.matrix.org/v1.12/client-server-api/#knocking-on-rooms)

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::{Pdu, generate_event_id, timestamp_ms};
use maelstrom_core::matrix::room::{JoinRule, Membership, event_type as et};
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/_matrix/client/v3/knock/{roomIdOrAlias}", post(knock_room))
}

#[derive(Deserialize)]
struct KnockRequest {
    #[serde(default)]
    reason: Option<String>,
}

/// POST /knock/{roomIdOrAlias} — knock on a room.
async fn knock_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id_or_alias): Path<String>,
    MatrixJson(body): MatrixJson<KnockRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Resolve alias if needed
    let room_id = if room_id_or_alias.starts_with('#') {
        storage
            .get_room_alias(&room_id_or_alias)
            .await
            .map_err(|e| match e {
                StorageError::NotFound => MatrixError::not_found("Room alias not found"),
                other => crate::extractors::storage_error(other),
            })?
    } else {
        room_id_or_alias
    };

    // Verify room exists
    storage.get_room(&room_id).await.map_err(|e| match e {
        StorageError::NotFound => MatrixError::not_found("Room not found"),
        other => crate::extractors::storage_error(other),
    })?;

    // Check join rules — knocking is only valid for rooms with join_rule "knock" or "knock_restricted"
    let join_rule = storage
        .get_state_event(&room_id, et::JOIN_RULES, "")
        .await
        .ok()
        .and_then(|e| {
            e.content
                .get("join_rule")
                .and_then(|j| j.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| JoinRule::Invite.as_str().to_string());

    if join_rule != JoinRule::Knock.as_str() && join_rule != JoinRule::KnockRestricted.as_str() {
        return Err(MatrixError::forbidden(
            "Room does not accept knocks (join_rule must be 'knock' or 'knock_restricted')",
        ));
    }

    // Check if already a member
    if let Ok(membership) = storage.get_membership(&sender, &room_id).await {
        match membership.as_str() {
            m if m == Membership::Join.as_str() => {
                return Err(MatrixError::forbidden("Already a member of this room"));
            }
            m if m == Membership::Ban.as_str() => {
                return Err(MatrixError::forbidden("You are banned from this room"));
            }
            m if m == Membership::Knock.as_str() => {
                // Already knocked — return success
                return Ok(Json(serde_json::json!({ "room_id": room_id })));
            }
            _ => {}
        }
    }

    // Create m.room.member event with membership: "knock"
    let mut content = serde_json::json!({ "membership": Membership::Knock.as_str() });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::json!(reason);
    }

    let event_id = generate_event_id();
    let pos = storage
        .next_stream_position()
        .await
        .map_err(crate::extractors::storage_error)?;

    let event = Pdu {
        event_id: event_id.clone(),
        room_id: room_id.clone(),
        sender: sender.clone(),
        event_type: et::MEMBER.to_string(),
        state_key: Some(sender.clone()),
        content,
        origin_server_ts: timestamp_ms(),
        unsigned: None,
        stream_position: pos,
        origin: None,
        auth_events: None,
        prev_events: None,
        depth: None,
        hashes: None,
        signatures: None,
    };

    storage
        .store_event(&event)
        .await
        .map_err(crate::extractors::storage_error)?;

    storage
        .set_room_state(&room_id, et::MEMBER, &sender, &event_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    storage
        .set_membership(&sender, &room_id, Membership::Knock.as_str())
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({ "room_id": room_id })))
}
