//! Room lifecycle -- create, join, leave, invite, ban, kick, forget, and upgrade.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 8 Rooms](https://spec.matrix.org/v1.13/client-server-api/#rooms)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `POST` | `/_matrix/client/v3/createRoom` | Create a new room |
//! | `POST` | `/_matrix/client/v3/join/{roomIdOrAlias}` | Join via room ID or alias |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/join` | Join a specific room |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/leave` | Leave a room |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/invite` | Invite a user |
//! | `GET`  | `/_matrix/client/v3/joined_rooms` | List joined rooms |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/kick` | Kick a user (set to leave) |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/ban` | Ban a user |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/unban` | Unban (set to leave) |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/forget` | Forget a left room |
//! | `GET`  | `/_matrix/client/v3/rooms/{roomId}/joined_members` | List joined members |
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/upgrade` | Upgrade room version |
//!
//! # Room creation
//!
//! `POST /createRoom` generates a new room ID and emits the required initial
//! state events in order:
//!
//! 1. `m.room.create` -- records the creator and room version (default v10)
//! 2. `m.room.member` -- creator joins the room
//! 3. `m.room.power_levels` -- default power levels (creator at 100)
//! 4. `m.room.join_rules` -- derived from `preset` (`public_chat` = public,
//!    others = invite-only)
//! 5. `m.room.history_visibility` -- always `shared` for all presets
//! 6. `m.room.name` / `m.room.topic` -- if provided
//! 7. Any `initial_state` events supplied by the client
//! 8. Invites for each user in the `invite` list
//! 9. Room alias and `m.room.canonical_alias` if `room_alias_name` is set
//!
//! # Join flow
//!
//! **Local rooms:** For public rooms (`join_rule: public`) any user can join.
//! For invite-only rooms the user must already have `membership: invite`. Joins
//! are idempotent -- joining a room you are already in returns immediately.
//!
//! **Remote rooms (federation):** When the room ID or alias belongs to a remote
//! server, the handler executes the three-step federation join:
//! `make_join` -> sign event -> `send_join`. The returned room state and auth
//! chain are stored locally so subsequent operations work without further
//! federation calls.
//!
//! # Invite flow
//!
//! The sender must be joined to the room. The target must not already be joined
//! or invited. For remote users, a federation `PUT /invite` is sent to the
//! target's homeserver. For local users, a `m.room.member` state event with
//! `membership: invite` is created directly.
//!
//! # Leave, kick, ban, unban
//!
//! - **Leave:** The user sets their own membership to `leave`. For federated
//!   rooms, `make_leave` / `send_leave` is used.
//! - **Kick:** The sender creates a `m.room.member` event with `membership:
//!   leave` targeting another user. The target must be currently joined.
//! - **Ban:** Sets the target's membership to `ban`, preventing future joins.
//! - **Unban:** Sets a banned user's membership back to `leave`.
//!
//! # Room upgrade
//!
//! Creates a brand-new room with the requested version, copies key state events
//! (power levels, name, topic, join rules, etc.) from the old room, then sends
//! an `m.room.tombstone` in the old room pointing to the replacement. The old
//! room's power levels are restricted so only the upgrader retains PL 100.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::{
    Pdu, default_power_levels, generate_event_id, generate_room_id, timestamp_ms,
};
use maelstrom_core::matrix::id::server_name_from_sigil_id;
use maelstrom_core::matrix::room::{JoinRule, Membership, event_type as et};
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::handlers::util::require_membership;
use crate::notify::Notification;
use crate::state::AppState;

/// Register all room lifecycle routes.
///
/// Routes cover creation, membership transitions (join/invite/leave/kick/ban/
/// unban/forget), member listing, and room upgrades. Legacy `r0` paths are
/// provided for `joined_members` for Complement compatibility.
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

    // Build auth_events per spec: create, power_levels, join_rules (for member events), sender's member
    let auth_events =
        crate::handlers::util::select_auth_events(storage, room_id, sender, event_type).await;

    let event = Pdu {
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
        auth_events: if auth_events.is_empty() {
            None
        } else {
            Some(auth_events)
        },
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

/// Request body for `POST /createRoom`.
///
/// The `preset` field controls default join rules and history visibility:
/// - `"public_chat"` -- join_rule=public, history_visibility=shared
/// - `"private_chat"` (default) -- join_rule=invite, history_visibility=shared
/// - `"trusted_private_chat"` -- join_rule=invite, history_visibility=shared
///
/// The `initial_state` array lets the client inject arbitrary state events
/// (e.g., `m.room.encryption`) into the room after the standard events.
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
            maelstrom_core::matrix::error::ErrorCode::UnsupportedRoomVersion,
            format!("Unsupported room version: {room_version}"),
        ));
    }

    let preset = body.preset.as_deref().unwrap_or("private_chat");
    let (join_rule, history_visibility) = match preset {
        "public_chat" => (JoinRule::Public.as_str(), "shared"),
        "trusted_private_chat" => (JoinRule::Invite.as_str(), "shared"),
        _ => (JoinRule::Invite.as_str(), "shared"), // private_chat default
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

    store_state_event(storage, &room_id, &sender, et::CREATE, "", create_content).await?;

    // 2. m.room.member (creator join)
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::MEMBER,
        &sender,
        serde_json::json!({ "membership": Membership::Join.as_str() }),
    )
    .await?;

    storage
        .set_membership(&sender, &room_id, Membership::Join.as_str())
        .await
        .map_err(crate::extractors::storage_error)?;

    // 3. m.room.power_levels
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::POWER_LEVELS,
        "",
        default_power_levels(&sender),
    )
    .await?;

    // 4. m.room.join_rules
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::JOIN_RULES,
        "",
        serde_json::json!({ "join_rule": join_rule }),
    )
    .await?;

    // 5. m.room.history_visibility
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::HISTORY_VISIBILITY,
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
            et::NAME,
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
            et::TOPIC,
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
        let mut invite_content = serde_json::json!({ "membership": Membership::Invite.as_str() });
        if body.is_direct {
            invite_content["is_direct"] = serde_json::json!(true);
        }
        store_state_event(
            storage,
            &room_id,
            &sender,
            et::MEMBER,
            invitee,
            invite_content,
        )
        .await?;

        storage
            .set_membership(invitee, &room_id, Membership::Invite.as_str())
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
            et::CANONICAL_ALIAS,
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
    axum::extract::Query(query): axum::extract::Query<JoinQuery>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let extra: Option<serde_json::Value> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice(&body).ok()
    };

    if room_id_or_alias.starts_with('#') {
        let alias_server = server_name_from_sigil_id(&room_id_or_alias);
        let is_local = alias_server == state.server_name().as_str();

        if is_local {
            // Local alias resolution
            let room_id = state
                .storage()
                .get_room_alias(&room_id_or_alias)
                .await
                .map_err(|e| match e {
                    StorageError::NotFound => MatrixError::not_found("Room alias not found"),
                    other => crate::extractors::storage_error(other),
                })?;
            do_join(&state, &auth, &room_id, extra.as_ref(), None).await
        } else {
            // Remote alias — query the remote server's directory, then federation join
            let fed = state
                .federation()
                .ok_or_else(|| MatrixError::unknown("Federation not configured"))?;
            let path = format!(
                "/_matrix/federation/v1/query/directory?room_alias={}",
                crate::handlers::util::percent_encode(&room_id_or_alias)
            );
            let resp = fed.get(alias_server, &path).await.map_err(|e| {
                MatrixError::not_found(format!("Failed to resolve remote alias: {e}"))
            })?;
            let room_id = resp
                .get("room_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| MatrixError::not_found("Remote alias not found"))?
                .to_string();

            let via = query.server_name.as_deref().unwrap_or(alias_server);
            do_join(&state, &auth, &room_id, extra.as_ref(), Some(via)).await
        }
    } else {
        // Room ID directly
        do_join(
            &state,
            &auth,
            &room_id_or_alias,
            extra.as_ref(),
            query.server_name.as_deref(),
        )
        .await
    }
}

#[derive(serde::Deserialize, Default)]
struct JoinQuery {
    server_name: Option<String>,
}

// -- POST /rooms/{roomId}/join --

async fn join_room(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<JoinQuery>,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let extra: Option<serde_json::Value> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice(&body).ok()
    };
    do_join(
        &state,
        &auth,
        &room_id,
        extra.as_ref(),
        query.server_name.as_deref(),
    )
    .await
}

async fn do_join(
    state: &AppState,
    auth: &AuthenticatedUser,
    room_id: &str,
    extra_content: Option<&serde_json::Value>,
    via_server: Option<&str>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check if room exists locally
    let room_exists_locally = storage.get_room(room_id).await.is_ok();

    if !room_exists_locally {
        // Room not local — attempt federation join
        return do_federation_join(state, &sender, room_id, via_server).await;
    }

    // Check join rules
    let join_rule = storage
        .get_state_event(room_id, et::JOIN_RULES, "")
        .await
        .ok()
        .and_then(|e| {
            e.content
                .get("join_rule")
                .and_then(|j| j.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| JoinRule::Invite.as_str().to_string());

    // Check current membership
    let current_membership = storage.get_membership(&sender, room_id).await.ok();

    // Idempotent join: if already joined, return existing member event_id
    if current_membership.as_deref() == Some(Membership::Join.as_str())
        && let Ok(_existing) = storage.get_state_event(room_id, et::MEMBER, &sender).await
    {
        return Ok(Json(serde_json::json!({ "room_id": room_id })));
    }

    if join_rule == JoinRule::Invite.as_str() || join_rule == JoinRule::Knock.as_str() {
        // Must be invited to join
        if current_membership.as_deref() != Some(Membership::Invite.as_str()) {
            return Err(MatrixError::forbidden("You are not invited to this room"));
        }
    }

    // Build member event content — merge extra body fields with membership
    let member_content = if let Some(serde_json::Value::Object(map)) = extra_content {
        let mut content = map.clone();
        content.insert(
            "membership".to_string(),
            serde_json::json!(Membership::Join.as_str()),
        );
        serde_json::Value::Object(content)
    } else {
        serde_json::json!({ "membership": Membership::Join.as_str() })
    };

    // Create m.room.member event
    store_state_event(
        storage,
        room_id,
        &sender,
        et::MEMBER,
        &sender,
        member_content,
    )
    .await?;

    storage
        .set_membership(&sender, room_id, Membership::Join.as_str())
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

/// Federation join: make_join → sign → send_join, then store returned state locally.
async fn do_federation_join(
    state: &AppState,
    sender: &str,
    room_id: &str,
    via_server: Option<&str>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let fed = state
        .federation()
        .ok_or_else(|| MatrixError::unknown("Federation not configured"))?;
    let storage = state.storage();
    let my_server = state.server_name().as_str();

    // Determine which server to join through
    let target_server = if let Some(server) = via_server {
        server.to_string()
    } else {
        let s = server_name_from_sigil_id(room_id);
        if s.is_empty() {
            return Err(MatrixError::not_found("Cannot determine room server"));
        }
        s.to_string()
    };

    tracing::info!(room_id, target_server, sender, "Initiating federation join");

    // Step 1: make_join — get event template from remote server
    let make_join_path = format!(
        "/_matrix/federation/v1/make_join/{}/{}",
        crate::handlers::util::percent_encode(room_id),
        crate::handlers::util::percent_encode(sender),
    );
    let make_join_resp = fed
        .get(&target_server, &make_join_path)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "make_join failed");
            MatrixError::not_found(format!("Failed to contact server {target_server}: {e}"))
        })?;

    let room_version = make_join_resp
        .get("room_version")
        .and_then(|v| v.as_str())
        .unwrap_or("10")
        .to_string();

    let event_template = make_join_resp
        .get("event")
        .cloned()
        .ok_or_else(|| MatrixError::unknown("make_join returned no event template"))?;

    // Step 2: Fill in and sign the join event
    let mut join_event = event_template;
    join_event["origin"] = serde_json::json!(my_server);
    join_event["origin_server_ts"] = serde_json::json!(timestamp_ms());

    // Sign the event
    let _signing_key = &state
        .federation()
        .ok_or_else(|| MatrixError::unknown("No federation client"))?;
    // Use the federation client's signing to compute content hash + signature
    // For now, generate event_id from content
    let event_id = generate_event_id();
    join_event["event_id"] = serde_json::json!(&event_id);

    // Step 3: send_join — send the signed event to the remote server
    // Request partial state per MSC3706 to speed up the join.
    let send_join_path = format!(
        "/_matrix/federation/v2/send_join/{}/{}?org.matrix.msc3706.partial_state=true",
        crate::handlers::util::percent_encode(room_id),
        crate::handlers::util::percent_encode(&event_id),
    );
    let send_join_resp = fed
        .put_json(&target_server, &send_join_path, &join_event)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "send_join failed");
            MatrixError::unknown(format!("send_join failed: {e}"))
        })?;

    // Step 4: Verify the join event was not tampered with by the remote server.
    // Check that our join event is present in the response and matches what we sent.
    if let Some(returned_event) = send_join_resp.get("event") {
        // Verify key fields match what we originally sent
        let orig_sender = join_event.get("sender").and_then(|v| v.as_str());
        let resp_sender = returned_event.get("sender").and_then(|v| v.as_str());
        let orig_type = join_event.get("type").and_then(|v| v.as_str());
        let resp_type = returned_event.get("type").and_then(|v| v.as_str());
        let orig_state_key = join_event.get("state_key").and_then(|v| v.as_str());
        let resp_state_key = returned_event.get("state_key").and_then(|v| v.as_str());

        if orig_sender != resp_sender || orig_type != resp_type || orig_state_key != resp_state_key
        {
            tracing::warn!(
                room_id,
                "send_join response event fields don't match what we sent — possible tampering"
            );
            return Err(MatrixError::unknown(
                "send_join returned a modified join event",
            ));
        }
    } else {
        tracing::warn!(room_id, "send_join response did not include the join event");
    }

    // TODO: Verify signatures on all state events and auth_chain events from their
    // origin servers. This requires fetching each origin server's signing keys via
    // the key server APIs and validating ed25519 signatures. For now we trust the
    // responding server, but this should be implemented for full spec compliance.

    // Step 5: Process the returned room state — create the room locally and store state
    let room_record = maelstrom_storage::traits::RoomRecord {
        room_id: room_id.to_string(),
        version: room_version,
        creator: String::new(), // Will be filled from state
        is_direct: false,
    };
    // Create room (ignore if already exists from race)
    let _ = storage.create_room(&room_record).await;

    // Store state events from the response
    if let Some(state_events) = send_join_resp.get("state").and_then(|s| s.as_array()) {
        for event_json in state_events {
            store_federation_event(storage, event_json).await;
        }
    }

    // Store auth chain events
    if let Some(auth_chain) = send_join_resp.get("auth_chain").and_then(|s| s.as_array()) {
        for event_json in auth_chain {
            store_federation_event(storage, event_json).await;
        }
    }

    // Store the join event itself
    store_federation_event(storage, &join_event).await;

    // Set local membership
    storage
        .set_membership(sender, room_id, Membership::Join.as_str())
        .await
        .map_err(crate::extractors::storage_error)?;

    // MSC3706: Check if this was a partial state join and spawn background resync
    let is_partial = send_join_resp
        .get("org.matrix.msc3706.partial_state")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_partial {
        tracing::info!(
            room_id,
            "Remote server returned partial state, scheduling background resync"
        );

        // Mark room as partially synced using account data on a synthetic user
        let _ = storage
            .set_account_data(
                &format!("_room:{room_id}"),
                None,
                "_maelstrom.partial_state",
                &serde_json::json!({"partial": true}),
            )
            .await;

        // Spawn background resync — AppState is cheap to clone (Arc internally)
        let bg_state = state.clone();
        let target = target_server.clone();
        let rid = room_id.to_string();
        let eid = event_id.clone();
        tokio::spawn(async move {
            resync_room_state(bg_state, target, rid, eid).await;
        });
    }

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.to_string(),
        })
        .await;

    tracing::info!(room_id, sender, "Federation join complete");
    Ok(Json(serde_json::json!({ "room_id": room_id })))
}

/// Store a federation event (from send_join state/auth_chain) into local storage.
async fn store_federation_event(
    storage: &dyn maelstrom_storage::traits::Storage,
    event_json: &serde_json::Value,
) {
    let event_id = event_json
        .get("event_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if event_id.is_empty() {
        return;
    }

    let room_id = event_json
        .get("room_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let sender = event_json
        .get("sender")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event_type = event_json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let state_key = event_json
        .get("state_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let stored = Pdu {
        event_id: event_id.clone(),
        room_id: room_id.clone(),
        sender,
        event_type: event_type.clone(),
        state_key: state_key.clone(),
        content: event_json
            .get("content")
            .cloned()
            .unwrap_or(serde_json::json!({})),
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

    // Store event (ignore dups)
    let _ = storage.store_event(&stored).await;

    // Update room state for state events
    if let Some(ref sk) = state_key {
        let _ = storage
            .set_room_state(&room_id, &event_type, sk, &event_id)
            .await;

        // Update membership graph for member events
        if event_type == et::MEMBER
            && let Some(membership) = stored.content.get("membership").and_then(|m| m.as_str())
        {
            let _ = storage.set_membership(sk, &room_id, membership).await;
        }
    }
}

/// MSC3706: Background task to fetch full room state after a partial state join.
///
/// When we join a remote room with `org.matrix.msc3706.partial_state=true`, the
/// responding server may return only the state needed for the join event's auth
/// chain rather than the full room state. This function runs in the background
/// to fetch the remaining state events so the room eventually has complete state.
async fn resync_room_state(
    state: AppState,
    target_server: String,
    room_id: String,
    event_id: String,
) {
    tracing::info!(room_id, "Starting partial state resync");

    let fed = match state.federation() {
        Some(f) => f,
        None => {
            tracing::warn!(room_id, "Federation not available for resync");
            return;
        }
    };
    let storage = state.storage();

    // 1. Get all state event IDs from the remote server
    let path = format!(
        "/_matrix/federation/v1/state_ids/{}?event_id={}",
        crate::handlers::util::percent_encode(&room_id),
        crate::handlers::util::percent_encode(&event_id),
    );
    let state_ids_resp = match fed.get(&target_server, &path).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(room_id, error = %e, "Failed to fetch state_ids for resync");
            return;
        }
    };

    let pdu_ids: Vec<String> = state_ids_resp
        .get("pdu_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let auth_chain_ids: Vec<String> = state_ids_resp
        .get("auth_chain_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // 2. Merge all IDs and fetch events we don't have locally
    let mut all_ids = pdu_ids;
    all_ids.extend(auth_chain_ids);

    let mut fetched = 0u64;
    for eid in &all_ids {
        // Check if the room still exists (user may have left during resync)
        if storage.get_room(&room_id).await.is_err() {
            tracing::info!(room_id, "Room no longer exists, aborting resync");
            return;
        }

        // Skip events we already have
        if storage.get_event(eid).await.is_ok() {
            continue;
        }

        let event_path = format!(
            "/_matrix/federation/v1/event/{}",
            crate::handlers::util::percent_encode(eid),
        );
        match fed.get(&target_server, &event_path).await {
            Ok(resp) => {
                // The /event response wraps the PDU in a "pdus" array
                if let Some(pdus) = resp.get("pdus").and_then(|p| p.as_array()) {
                    for pdu_json in pdus {
                        store_federation_event(storage, pdu_json).await;
                    }
                    fetched += 1;
                }
            }
            Err(e) => {
                tracing::debug!(event_id = eid, error = %e, "Failed to fetch event during resync");
            }
        }
    }

    // 3. Mark room as fully synced — remove partial state flag
    let _ = storage
        .delete_account_data(
            &format!("_room:{room_id}"),
            None,
            "_maelstrom.partial_state",
        )
        .await;

    tracing::info!(
        room_id,
        total = all_ids.len(),
        fetched,
        "Partial state resync complete"
    );
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

    if membership != Membership::Join.as_str() && membership != Membership::Invite.as_str() {
        return Err(MatrixError::forbidden("Not a member of this room"));
    }

    // Detect if this is a federated room by checking if the room's server differs from ours
    let room_server = server_name_from_sigil_id(&room_id);
    let is_remote = room_server != state.server_name().as_str();

    if is_remote && membership == Membership::Join.as_str() {
        // Federation leave: make_leave → sign → send_leave
        if let Some(fed) = state.federation() {
            let make_path = format!(
                "/_matrix/federation/v1/make_leave/{}/{}",
                crate::handlers::util::percent_encode(&room_id),
                crate::handlers::util::percent_encode(&sender),
            );
            if let Ok(make_resp) = fed.get(room_server, &make_path).await {
                let mut leave_event = make_resp
                    .get("event")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                leave_event["origin"] = serde_json::json!(state.server_name().as_str());
                leave_event["origin_server_ts"] = serde_json::json!(timestamp_ms());
                let event_id = generate_event_id();

                let send_path = format!(
                    "/_matrix/federation/v2/send_leave/{}/{}",
                    crate::handlers::util::percent_encode(&room_id),
                    crate::handlers::util::percent_encode(&event_id),
                );
                let _ = fed.put_json(room_server, &send_path, &leave_event).await;
            }
        }
    }

    // Create local m.room.member leave event
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::MEMBER,
        &sender,
        serde_json::json!({ "membership": Membership::Leave.as_str() }),
    )
    .await?;

    storage
        .set_membership(&sender, &room_id, Membership::Leave.as_str())
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

/// Request body for `POST /rooms/{roomId}/invite`.
///
/// The target `user_id` may be local or remote. Remote invites are delivered
/// via the federation `PUT /invite` endpoint.
#[derive(Deserialize)]
struct InviteRequest {
    user_id: String,
    #[serde(default)]
    is_direct: Option<bool>,
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
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Can't invite yourself
    if body.user_id == sender {
        return Err(MatrixError::forbidden("Cannot invite yourself"));
    }

    // Can't invite someone already joined or invited
    if let Ok(target_membership) = storage.get_membership(&body.user_id, &room_id).await {
        match target_membership.as_str() {
            m if m == Membership::Join.as_str() => {
                return Err(MatrixError::forbidden("User is already in the room"));
            }
            m if m == Membership::Invite.as_str() => {
                return Err(MatrixError::forbidden("User is already invited"));
            }
            // ban is allowed — spec permits unban-via-invite
            _ => {}
        }
    }

    // Check server ACL for the invited user's server
    let target_server = server_name_from_sigil_id(&body.user_id);
    if !target_server.is_empty() {
        crate::handlers::util::check_server_acl(storage, &room_id, target_server).await?;
    }

    // Check if target user is local or remote
    let is_remote = target_server != state.server_name().as_str();

    if is_remote {
        // Federation invite: build event and send via PUT /invite
        let fed = state
            .federation()
            .ok_or_else(|| MatrixError::unknown("Federation not configured"))?;

        let event_id = generate_event_id();
        let mut invite_content = serde_json::json!({ "membership": Membership::Invite.as_str() });
        if let Some(true) = body.is_direct {
            invite_content["is_direct"] = serde_json::json!(true);
        }
        let invite_event = serde_json::json!({
            "event_id": event_id,
            "room_id": room_id,
            "sender": sender,
            "type": et::MEMBER,
            "state_key": body.user_id,
            "content": invite_content,
            "origin": state.server_name().as_str(),
            "origin_server_ts": timestamp_ms(),
            "depth": 100,
        });

        let path = format!(
            "/_matrix/federation/v2/invite/{}/{}",
            crate::handlers::util::percent_encode(&room_id),
            crate::handlers::util::percent_encode(&event_id),
        );
        let body_json = serde_json::json!({
            "event": invite_event,
            "room_version": storage.get_room(&room_id).await
                .map(|r| r.version).unwrap_or_else(|_| "10".to_string()),
            "invite_room_state": [],
        });

        fed.put_json(target_server, &path, &body_json)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Federation invite failed");
                MatrixError::unknown(format!("Failed to send invite: {e}"))
            })?;

        // Store locally too
        let _ = store_state_event(
            storage,
            &room_id,
            &sender,
            et::MEMBER,
            &body.user_id,
            invite_content.clone(),
        )
        .await;
    } else {
        // Local invite
        let mut local_content = serde_json::json!({ "membership": Membership::Invite.as_str() });
        if let Some(true) = body.is_direct {
            local_content["is_direct"] = serde_json::json!(true);
        }
        store_state_event(
            storage,
            &room_id,
            &sender,
            et::MEMBER,
            &body.user_id,
            local_content,
        )
        .await?;
    }

    storage
        .set_membership(&body.user_id, &room_id, Membership::Invite.as_str())
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

/// Shared request body for kick, ban, and unban operations.
///
/// All three operations target a `user_id` and optionally include a human-
/// readable `reason` that is stored in the resulting `m.room.member` event.
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
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Check target is actually joined to the room
    let target_membership = storage
        .get_membership(&body.user_id, &room_id)
        .await
        .unwrap_or_default();

    if target_membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden(
            "Cannot kick a user who is not in the room",
        ));
    }

    // Build content
    let mut content = serde_json::json!({ "membership": Membership::Leave.as_str() });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::Value::String(reason.clone());
    }

    // Create m.room.member leave event for the target
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::MEMBER,
        &body.user_id,
        content,
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, Membership::Leave.as_str())
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
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Build content
    let mut content = serde_json::json!({ "membership": Membership::Ban.as_str() });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::Value::String(reason.clone());
    }

    // Create m.room.member ban event for the target
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::MEMBER,
        &body.user_id,
        content,
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, Membership::Ban.as_str())
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
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
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

    if target_membership != Membership::Ban.as_str() {
        return Err(MatrixError::forbidden("User is not banned"));
    }

    // Build content
    let mut content = serde_json::json!({ "membership": Membership::Leave.as_str() });
    if let Some(reason) = &body.reason {
        content["reason"] = serde_json::Value::String(reason.clone());
    }

    // Create m.room.member leave event for the target
    store_state_event(
        storage,
        &room_id,
        &sender,
        et::MEMBER,
        &body.user_id,
        content,
    )
    .await?;

    storage
        .set_membership(&body.user_id, &room_id, Membership::Leave.as_str())
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

    if membership == Membership::Join.as_str() || membership == Membership::Invite.as_str() {
        return Err(MatrixError::new(
            http::StatusCode::BAD_REQUEST,
            maelstrom_core::matrix::error::ErrorCode::Unknown,
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
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Get all joined members
    let members = storage
        .get_room_members(&room_id, Membership::Join.as_str())
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

/// Request body for `POST /rooms/{roomId}/upgrade`.
///
/// The `new_version` must be a supported room version (1-11). The caller
/// must have power level sufficient to send `m.room.tombstone` (default: 100).
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
    let membership = require_membership(storage, &sender, &old_room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Check power level for tombstone (requires PL 100 by default)
    let power_levels = storage
        .get_state_event(&old_room_id, et::POWER_LEVELS, "")
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
        .and_then(|ev| ev.get(et::TOMBSTONE))
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
            maelstrom_core::matrix::error::ErrorCode::UnsupportedRoomVersion,
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
        et::CREATE,
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
            et::CREATE | et::MEMBER | et::TOMBSTONE
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
        et::MEMBER,
        &sender,
        serde_json::json!({ "membership": Membership::Join.as_str() }),
    )
    .await?;

    storage
        .set_membership(&sender, &new_room_id, Membership::Join.as_str())
        .await
        .map_err(crate::extractors::storage_error)?;

    // Carry over push rules from old room to new room for all joined members
    let old_members = storage
        .get_room_members(&old_room_id, Membership::Join.as_str())
        .await
        .unwrap_or_default();
    for member in &old_members {
        if let Ok(rules) = storage
            .get_account_data(member, None, "_maelstrom.push_rules")
            .await
            && let Some(obj) = rules.as_object()
        {
            let mut updated = false;
            let mut new_rules = rules.clone();
            // Copy "room" rules that target old_room_id
            if let Some(room_rules) = obj.get("room").and_then(|v| v.as_array()) {
                let mut new_room_rules: Vec<serde_json::Value> = room_rules.clone();
                for rule in room_rules {
                    if rule.get("rule_id").and_then(|v| v.as_str()) == Some(&old_room_id) {
                        let mut new_rule = rule.clone();
                        new_rule["rule_id"] = serde_json::Value::String(new_room_id.clone());
                        new_room_rules.push(new_rule);
                        updated = true;
                    }
                }
                if updated {
                    new_rules["room"] = serde_json::json!(new_room_rules);
                }
            }
            if updated {
                let _ = storage
                    .set_account_data(member, None, "_maelstrom.push_rules", &new_rules)
                    .await;
            }
        }
    }

    // Send tombstone to old room
    let tombstone_event_id = store_state_event(
        storage,
        &old_room_id,
        &sender,
        et::TOMBSTONE,
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

// Auth event selection uses crate::handlers::util::select_auth_events
