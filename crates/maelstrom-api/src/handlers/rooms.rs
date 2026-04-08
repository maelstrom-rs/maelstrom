use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::error::MatrixError;
use maelstrom_core::events::pdu::{
    StoredEvent, default_power_levels, generate_event_id, generate_room_id, timestamp_ms,
};
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/createRoom", post(create_room))
        .route(
            "/_matrix/client/v3/join/{roomIdOrAlias}",
            post(join_room_by_alias),
        )
        .route("/_matrix/client/v3/rooms/{roomId}/join", post(join_room))
        .route("/_matrix/client/v3/rooms/{roomId}/leave", post(leave_room))
        .route(
            "/_matrix/client/v3/rooms/{roomId}/invite",
            post(invite_to_room),
        )
        .route("/_matrix/client/v3/joined_rooms", get(joined_rooms))
        .route("/_matrix/client/v3/rooms/{roomId}/kick", post(kick_user))
        .route("/_matrix/client/v3/rooms/{roomId}/ban", post(ban_user))
        .route("/_matrix/client/v3/rooms/{roomId}/unban", post(unban_user))
        .route(
            "/_matrix/client/v3/rooms/{roomId}/forget",
            post(forget_room),
        )
        .route(
            "/_matrix/client/v3/rooms/{roomId}/joined_members",
            get(joined_members),
        )
        .route(
            "/_matrix/client/v3/rooms/{roomId}/upgrade",
            post(upgrade_room),
        )
        // r0 compat
        .route(
            "/_matrix/client/r0/rooms/{roomId}/joined_members",
            get(joined_members),
        )
}

/// Helper to create, store, and register a state event in one step.
/// Reduces repetition in create_room and similar flows.
async fn store_state_event(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    sender: &str,
    event_type: &str,
    state_key: &str,
    content: serde_json::Value,
) -> Result<String, MatrixError> {
    let event_id = generate_event_id();
    let event = StoredEvent {
        event_id: event_id.clone(),
        room_id: room_id.to_string(),
        sender: sender.to_string(),
        event_type: event_type.to_string(),
        state_key: Some(state_key.to_string()),
        content,
        origin_server_ts: timestamp_ms(),
        unsigned: None,
        stream_position: 0, // Set by store_event()
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
        .map_err(|e| {
            tracing::error!(event_type = %event_type, room_id = %room_id, error = %e, "Failed to store state event");
            crate::extractors::storage_error(e)
        })?;
    storage
        .set_room_state(room_id, event_type, state_key, &event_id)
        .await
        .map_err(crate::extractors::storage_error)?;
    Ok(event_id)
}

// -- POST /createRoom --

#[derive(Deserialize)]
#[allow(dead_code)]
struct CreateRoomRequest {
    #[serde(default)]
    visibility: Option<String>,
    name: Option<String>,
    topic: Option<String>,
    preset: Option<String>,
    #[serde(default)]
    invite: Vec<String>,
    #[serde(default)]
    is_direct: bool,
    room_version: Option<String>,
    room_alias_name: Option<String>,
    #[serde(default)]
    creation_content: Option<serde_json::Value>,
    #[serde(default)]
    initial_state: Vec<InitialStateEvent>,
}

#[derive(Deserialize)]
struct InitialStateEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    state_key: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct CreateRoomResponse {
    room_id: String,
}

async fn create_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<CreateRoomRequest>,
) -> Result<Json<CreateRoomResponse>, MatrixError> {
    let server_name = state.server_name().as_str();
    let room_id = generate_room_id(server_name);
    let sender = auth.user_id.to_string();
    let room_version = body.room_version.unwrap_or_else(|| "10".to_string());

    // Validate room version
    let known_versions = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];
    if !known_versions.contains(&room_version.as_str()) {
        return Err(MatrixError::new(
            http::StatusCode::BAD_REQUEST,
            maelstrom_core::error::ErrorCode::UnsupportedRoomVersion,
            format!("Unsupported room version: {room_version}"),
        ));
    }

    let preset = body.preset.as_deref().unwrap_or("private_chat");
    let (join_rule, history_visibility) = match preset {
        "public_chat" => ("public", "shared"),
        "trusted_private_chat" => ("invite", "shared"),
        _ => ("invite", "shared"), // private_chat default
    };

    // Create room record
    let room_record = maelstrom_storage::traits::RoomRecord {
        room_id: room_id.clone(),
        version: room_version.clone(),
        creator: sender.clone(),
        is_direct: body.is_direct,
    };

    state
        .storage()
        .create_room(&room_record)
        .await
        .map_err(crate::extractors::storage_error)?;

    let storage = state.storage();

    // 1. m.room.create — merge creation_content but never allow overriding room_version
    let mut create_content = if let Some(serde_json::Value::Object(map)) = body.creation_content {
        let mut base = serde_json::Map::from_iter(map);
        // room_version must not be overridden via creation_content
        base.remove("room_version");
        serde_json::Value::Object(base)
    } else {
        serde_json::json!({})
    };
    create_content["creator"] = serde_json::json!(sender);
    create_content["room_version"] = serde_json::json!(room_version);

    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.create",
        "",
        create_content,
    )
    .await?;

    // 2. m.room.member (creator join)
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.member",
        &sender,
        serde_json::json!({ "membership": "join" }),
    )
    .await?;

    storage
        .set_membership(&sender, &room_id, "join")
        .await
        .map_err(crate::extractors::storage_error)?;

    // 3. m.room.power_levels
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.power_levels",
        "",
        default_power_levels(&sender),
    )
    .await?;

    // 4. m.room.join_rules
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.join_rules",
        "",
        serde_json::json!({ "join_rule": join_rule }),
    )
    .await?;

    // 5. m.room.history_visibility
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.history_visibility",
        "",
        serde_json::json!({ "history_visibility": history_visibility }),
    )
    .await?;

    // 6. m.room.name (if specified)
    if let Some(name) = &body.name {
        store_state_event(
            storage,
            &room_id,
            &sender,
            "m.room.name",
            "",
            serde_json::json!({ "name": name }),
        )
        .await?;
    }

    // 7. Additional initial_state events (before explicit topic so topic overrides)
    for is_event in &body.initial_state {
        store_state_event(
            storage,
            &room_id,
            &sender,
            &is_event.event_type,
            &is_event.state_key,
            is_event.content.clone(),
        )
        .await?;
    }

    // 8. m.room.topic (if specified — after initial_state to override any topic set there)
    if let Some(topic) = &body.topic {
        store_state_event(
            storage,
            &room_id,
            &sender,
            "m.room.topic",
            "",
            serde_json::json!({
                "topic": topic,
                "m.topic": { "m.text": [{ "body": topic }] },
            }),
        )
        .await?;
    }

    // 9. Process invites
    for invitee in &body.invite {
        store_state_event(
            storage,
            &room_id,
            &sender,
            "m.room.member",
            invitee,
            serde_json::json!({ "membership": "invite" }),
        )
        .await?;

        storage
            .set_membership(invitee, &room_id, "invite")
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // 10. Set room visibility
    if let Some(vis) = &body.visibility {
        storage
            .set_room_visibility(&room_id, vis)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // 11. Create room alias if room_alias_name is specified
    if let Some(alias_name) = &body.room_alias_name {
        let full_alias = format!("#{}:{}", alias_name, server_name);
        // Best-effort alias creation; ignore duplicates
        let _ = storage.set_room_alias(&full_alias, &room_id, &sender).await;

        // Set canonical alias state event
        store_state_event(
            storage,
            &room_id,
            &sender,
            "m.room.canonical_alias",
            "",
            serde_json::json!({ "alias": full_alias }),
        )
        .await?;
    }

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(CreateRoomResponse { room_id }))
}

// -- POST /join/{roomIdOrAlias} --

async fn join_room_by_alias(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id_or_alias): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let room_id = if room_id_or_alias.starts_with('#') {
        // Resolve alias to room_id
        state
            .storage()
            .get_room_alias(&room_id_or_alias)
            .await
            .map_err(|e| match e {
                StorageError::NotFound => MatrixError::not_found("Room alias not found"),
                other => crate::extractors::storage_error(other),
            })?
    } else {
        room_id_or_alias
    };

    let extra: Option<serde_json::Value> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice(&body).ok()
    };
    do_join(&state, &auth, &room_id, extra.as_ref()).await
}

// -- POST /rooms/{roomId}/join --

async fn join_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let extra: Option<serde_json::Value> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice(&body).ok()
    };
    do_join(&state, &auth, &room_id, extra.as_ref()).await
}

async fn do_join(
    state: &AppState,
    auth: &AuthenticatedUser,
    room_id: &str,
    extra_content: Option<&serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check room exists
    storage.get_room(room_id).await.map_err(|e| match e {
        StorageError::NotFound => MatrixError::not_found("Room not found"),
        other => crate::extractors::storage_error(other),
    })?;

    // Check join rules
    let join_rule = storage
        .get_state_event(room_id, "m.room.join_rules", "")
        .await
        .ok()
        .and_then(|e| {
            e.content
                .get("join_rule")
                .and_then(|j| j.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "invite".to_string());

    // Check current membership
    let current_membership = storage.get_membership(&sender, room_id).await.ok();

    // Idempotent join: if already joined, return existing member event_id
    if current_membership.as_deref() == Some("join")
        && let Ok(_existing) = storage
            .get_state_event(room_id, "m.room.member", &sender)
            .await
    {
        return Ok(Json(serde_json::json!({ "room_id": room_id })));
    }

    if join_rule == "invite" || join_rule == "knock" {
        // Must be invited to join
        if current_membership.as_deref() != Some("invite") {
            return Err(MatrixError::forbidden("You are not invited to this room"));
        }
    }

    // Build member event content — merge extra body fields with membership
    let member_content = if let Some(serde_json::Value::Object(map)) = extra_content {
        let mut content = map.clone();
        content.insert("membership".to_string(), serde_json::json!("join"));
        serde_json::Value::Object(content)
    } else {
        serde_json::json!({ "membership": "join" })
    };

    // Create m.room.member event
    store_state_event(
        storage,
        room_id,
        &sender,
        "m.room.member",
        &sender,
        member_content,
    )
    .await?;

    storage
        .set_membership(&sender, room_id, "join")
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.to_string(),
        })
        .await;

    Ok(Json(serde_json::json!({ "room_id": room_id })))
}

// -- POST /rooms/{roomId}/leave --

async fn leave_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is a member
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("Not a member of this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" && membership != "invite" {
        return Err(MatrixError::forbidden("Not a member of this room"));
    }

    // Create m.room.member leave event
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.member",
        &sender,
        serde_json::json!({ "membership": "leave" }),
    )
    .await?;

    storage
        .set_membership(&sender, &room_id, "leave")
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}

// -- POST /rooms/{roomId}/invite --

#[derive(Deserialize)]
struct InviteRequest {
    user_id: String,
}

async fn invite_to_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    MatrixJson(body): MatrixJson<InviteRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check sender is joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Can't invite yourself
    if body.user_id == sender {
        return Err(MatrixError::forbidden("Cannot invite yourself"));
    }

    // Can't invite someone already joined or invited
    if let Ok(target_membership) = storage.get_membership(&body.user_id, &room_id).await {
        match target_membership.as_str() {
            "join" => return Err(MatrixError::forbidden("User is already in the room")),
            "invite" => return Err(MatrixError::forbidden("User is already invited")),
            "ban" => return Err(MatrixError::forbidden("User is banned from this room")),
            _ => {}
        }
    }

    // Create invite event
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.member",
        &body.user_id,
        serde_json::json!({ "membership": "invite" }),
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, "invite")
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}

// -- GET /joined_rooms --

#[derive(Serialize)]
struct JoinedRoomsResponse {
    joined_rooms: Vec<String>,
}

async fn joined_rooms(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<JoinedRoomsResponse>, MatrixError> {
    let rooms = state
        .storage()
        .get_joined_rooms(auth.user_id.as_ref())
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(JoinedRoomsResponse {
        joined_rooms: rooms,
    }))
}

// -- POST /rooms/{roomId}/kick --

#[derive(Deserialize)]
struct KickBanRequest {
    user_id: String,
    #[serde(default)]
    reason: Option<String>,
}

async fn kick_user(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    MatrixJson(body): MatrixJson<KickBanRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check sender is joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Build content
    let mut content = serde_json::json!({ "membership": "leave" });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::Value::String(reason.clone());
    }

    // Create m.room.member leave event for the target
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.member",
        &body.user_id,
        content,
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, "leave")
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}

// -- POST /rooms/{roomId}/ban --

async fn ban_user(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    MatrixJson(body): MatrixJson<KickBanRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check sender is joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Build content
    let mut content = serde_json::json!({ "membership": "ban" });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::Value::String(reason.clone());
    }

    // Create m.room.member ban event for the target
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.member",
        &body.user_id,
        content,
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, "ban")
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}

// -- POST /rooms/{roomId}/unban --

async fn unban_user(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    MatrixJson(body): MatrixJson<KickBanRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check sender is joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Check target is banned
    let target_membership = storage
        .get_membership(&body.user_id, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("User not found in room"),
            other => crate::extractors::storage_error(other),
        })?;

    if target_membership != "ban" {
        return Err(MatrixError::forbidden("User is not banned"));
    }

    // Build content
    let mut content = serde_json::json!({ "membership": "leave" });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::Value::String(reason.clone());
    }

    // Create m.room.member leave event for the target
    store_state_event(
        storage,
        &room_id,
        &sender,
        "m.room.member",
        &body.user_id,
        content,
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, "leave")
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}

// -- POST /rooms/{roomId}/forget --

async fn forget_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user has left the room
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .unwrap_or_default();

    if membership == "join" || membership == "invite" {
        return Err(MatrixError::new(
            http::StatusCode::BAD_REQUEST,
            maelstrom_core::error::ErrorCode::Unknown,
            "You must leave the room before forgetting it",
        ));
    }

    storage
        .forget_room(&sender, &room_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

// -- GET /rooms/{roomId}/joined_members --

async fn joined_members(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is currently joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Get all joined members
    let members = storage
        .get_room_members(&room_id, "join")
        .await
        .map_err(crate::extractors::storage_error)?;

    let mut joined = serde_json::Map::new();
    for member in members {
        joined.insert(
            member,
            serde_json::json!({
                "display_name": null,
                "avatar_url": null,
            }),
        );
    }

    Ok(Json(serde_json::json!({ "joined": joined })))
}

// -- POST /rooms/{roomId}/upgrade --

#[derive(Deserialize)]
struct UpgradeRequest {
    new_version: String,
}

async fn upgrade_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(old_room_id): Path<String>,
    MatrixJson(body): MatrixJson<UpgradeRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();
    let server_name = state.server_name().as_str();

    // Validate the user is joined and has sufficient power level
    let membership = storage
        .get_membership(&sender, &old_room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Check power level for tombstone (requires PL 100 by default)
    let power_levels = storage
        .get_state_event(&old_room_id, "m.room.power_levels", "")
        .await
        .ok();

    let user_pl = power_levels
        .as_ref()
        .and_then(|e| e.content.get("users"))
        .and_then(|u| u.get(&sender))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let tombstone_pl = power_levels
        .as_ref()
        .and_then(|e| e.content.get("events"))
        .and_then(|ev| ev.get("m.room.tombstone"))
        .and_then(|v| v.as_i64())
        .unwrap_or(100);

    if user_pl < tombstone_pl {
        return Err(MatrixError::forbidden(
            "Insufficient power level to upgrade room",
        ));
    }

    // Validate room version
    let known_versions = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];
    if !known_versions.contains(&body.new_version.as_str()) {
        return Err(MatrixError::new(
            http::StatusCode::BAD_REQUEST,
            maelstrom_core::error::ErrorCode::UnsupportedRoomVersion,
            format!("Unsupported room version: {}", body.new_version),
        ));
    }

    // Create the new room
    let new_room_id = generate_room_id(server_name);

    let room_record = maelstrom_storage::traits::RoomRecord {
        room_id: new_room_id.clone(),
        version: body.new_version.clone(),
        creator: sender.clone(),
        is_direct: false,
    };

    storage
        .create_room(&room_record)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Create initial state in new room: m.room.create with predecessor
    store_state_event(
        storage,
        &new_room_id,
        &sender,
        "m.room.create",
        "",
        serde_json::json!({
            "creator": sender,
            "room_version": body.new_version,
            "predecessor": {
                "room_id": old_room_id,
                "event_id": "$tombstone", // Will be updated below
            },
        }),
    )
    .await?;

    // Copy key state from old room to new room
    let old_state = storage
        .get_current_state(&old_room_id)
        .await
        .unwrap_or_default();
    for event in &old_state {
        // Copy join_rules, history_visibility, power_levels, name, topic, etc.
        // Skip m.room.create (already set), m.room.member (will be handled), m.room.tombstone
        let dominated = matches!(
            event.event_type.as_str(),
            "m.room.create" | "m.room.member" | "m.room.tombstone"
        );
        if !dominated {
            store_state_event(
                storage,
                &new_room_id,
                &sender,
                &event.event_type,
                event.state_key.as_deref().unwrap_or(""),
                event.content.clone(),
            )
            .await?;
        }
    }

    // Join the creator to the new room
    store_state_event(
        storage,
        &new_room_id,
        &sender,
        "m.room.member",
        &sender,
        serde_json::json!({ "membership": "join" }),
    )
    .await?;

    storage
        .set_membership(&sender, &new_room_id, "join")
        .await
        .map_err(crate::extractors::storage_error)?;

    // Send tombstone to old room
    let tombstone_event_id = store_state_event(
        storage,
        &old_room_id,
        &sender,
        "m.room.tombstone",
        "",
        serde_json::json!({
            "body": "This room has been replaced",
            "replacement_room": new_room_id,
        }),
    )
    .await?;

    // Store the upgrade graph edge: old_room --upgrades_to--> new_room
    storage
        .store_room_upgrade(
            &old_room_id,
            &new_room_id,
            &body.new_version,
            &sender,
            &tombstone_event_id,
        )
        .await
        .map_err(crate::extractors::storage_error)?;

    // Notify
    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: old_room_id.clone(),
        })
        .await;
    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: new_room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({ "replacement_room": new_room_id })))
}
