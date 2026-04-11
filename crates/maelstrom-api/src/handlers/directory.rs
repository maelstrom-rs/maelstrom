//! Room directory -- aliases, visibility, and public room listing.
//!
//! The room directory lets users discover rooms by browsing or searching a
//! public list. Three concepts come together here:
//!
//! * **Room aliases** -- human-readable names like `#general:example.com` that
//!   resolve to an opaque room ID. A room can have many aliases; aliases can be
//!   created, looked up, and deleted.
//! * **Room visibility** -- controls whether a room appears in the server's
//!   public room list. This is separate from join rules; a room can be publicly
//!   listed but still require an invitation to join.
//! * **Public room list** -- the browseable/searchable directory of rooms whose
//!   visibility is set to `public`.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `PUT`    | `/_matrix/client/v3/directory/room/{roomAlias}` | Create a new room alias mapping |
//! | `GET`    | `/_matrix/client/v3/directory/room/{roomAlias}` | Resolve an alias to a room ID and servers |
//! | `DELETE` | `/_matrix/client/v3/directory/room/{roomAlias}` | Delete a room alias |
//! | `GET`    | `/_matrix/client/v3/rooms/{roomId}/aliases` | List all aliases for a room |
//! | `PUT`    | `/_matrix/client/v3/directory/list/room/{roomId}` | Set the visibility of a room in the directory |
//! | `GET`    | `/_matrix/client/v3/publicRooms` | Get the public room list (simple) |
//! | `POST`   | `/_matrix/client/v3/publicRooms` | Search/filter the public room list |
//!
//! # Matrix spec
//!
//! * [Room aliases](https://spec.matrix.org/v1.12/client-server-api/#room-aliases)
//! * [Listing rooms](https://spec.matrix.org/v1.12/client-server-api/#listing-rooms)

use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::server_name_from_sigil_id;
use maelstrom_core::matrix::room::{Membership, event_type as et};
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::handlers::util::require_membership;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/_matrix/client/v3/directory/room/{roomAlias}",
            put(set_room_alias)
                .get(get_room_alias)
                .delete(delete_room_alias),
        )
        .route(
            "/_matrix/client/v3/rooms/{roomId}/aliases",
            get(get_room_aliases),
        )
        .route(
            "/_matrix/client/v3/directory/list/room/{roomId}",
            put(set_room_visibility),
        )
        .route(
            "/_matrix/client/v3/publicRooms",
            get(get_public_rooms).post(search_public_rooms),
        )
}

// -- PUT /_matrix/client/v3/directory/room/{roomAlias} --

#[derive(Deserialize)]
struct SetAliasRequest {
    room_id: String,
}

async fn set_room_alias(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_alias): Path<String>,
    MatrixJson(body): MatrixJson<SetAliasRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Verify the room exists
    storage.get_room(&body.room_id).await.map_err(|e| match e {
        StorageError::NotFound => MatrixError::not_found("Room not found"),
        other => crate::extractors::storage_error(other),
    })?;

    storage
        .set_room_alias(&room_alias, &body.room_id, &sender)
        .await
        .map_err(|e| match e {
            StorageError::Duplicate(_) => MatrixError::new(
                http::StatusCode::CONFLICT,
                maelstrom_core::matrix::error::ErrorCode::Unknown,
                "Alias already in use",
            ),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(serde_json::json!({})))
}

// -- GET /_matrix/client/v3/directory/room/{roomAlias} --

async fn get_room_alias(
    State(state): State<AppState>,
    Path(room_alias): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let alias_server = server_name_from_sigil_id(&room_alias);
    let local_server = state.server_name().as_str();

    // If the alias belongs to a remote server, query it via federation
    if !alias_server.is_empty() && alias_server != local_server {
        let fed = state
            .federation()
            .ok_or_else(|| MatrixError::not_found("Room alias not found"))?;

        let encoded_alias = crate::handlers::util::percent_encode(&room_alias);
        let path = format!(
            "/_matrix/federation/v1/query/directory?room_alias={}",
            encoded_alias,
        );

        let response = fed.get(alias_server, &path).await.map_err(|e| {
            tracing::warn!(error = %e, alias = %room_alias, "Federation directory query failed");
            MatrixError::not_found("Room alias not found")
        })?;

        return Ok(Json(response));
    }

    // Local alias lookup
    let storage = state.storage();

    let room_id = storage
        .get_room_alias(&room_alias)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Room alias not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(serde_json::json!({
        "room_id": room_id,
        "servers": [local_server],
    })))
}

// -- DELETE /_matrix/client/v3/directory/room/{roomAlias} --

async fn delete_room_alias(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_alias): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Get the room this alias points to
    let room_id = storage
        .get_room_alias(&room_alias)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Room alias not found"),
            other => crate::extractors::storage_error(other),
        })?;

    // Check if user is the alias creator or has sufficient power level
    let is_creator = storage
        .get_room_alias_creator(&room_alias)
        .await
        .map(|c| c == sender)
        .unwrap_or(false);

    if !is_creator {
        // Check power level — need state_default or higher
        let power_levels = storage
            .get_state_event(&room_id, et::POWER_LEVELS, "")
            .await
            .ok();

        let user_pl = power_levels
            .as_ref()
            .and_then(|e| e.content.get("users"))
            .and_then(|u| u.get(&sender))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let required_pl = power_levels
            .as_ref()
            .and_then(|e| e.content.get("state_default"))
            .and_then(|v| v.as_i64())
            .unwrap_or(50);

        if user_pl < required_pl {
            return Err(MatrixError::forbidden(
                "You do not have permission to delete this alias",
            ));
        }
    }

    storage
        .delete_room_alias(&room_alias)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Room alias not found"),
            other => crate::extractors::storage_error(other),
        })?;

    // If this was the canonical alias, clear the m.room.canonical_alias state event
    if let Ok(canonical_event) = storage
        .get_state_event(&room_id, et::CANONICAL_ALIAS, "")
        .await
    {
        let is_canonical = canonical_event
            .content
            .get("alias")
            .and_then(|a| a.as_str())
            == Some(&room_alias);
        if is_canonical {
            use maelstrom_core::matrix::event::{Pdu, generate_event_id, timestamp_ms};
            let event_id = generate_event_id();
            let auth_events = crate::handlers::util::select_auth_events(
                storage,
                &room_id,
                &sender,
                et::CANONICAL_ALIAS,
            )
            .await;
            let event = Pdu {
                event_id: event_id.clone(),
                room_id: room_id.clone(),
                sender: sender.clone(),
                event_type: et::CANONICAL_ALIAS.to_string(),
                state_key: Some(String::new()),
                content: serde_json::json!({}),
                origin_server_ts: timestamp_ms(),
                unsigned: None,
                stream_position: 0,
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
            if let Err(e) = storage.store_event(&event).await {
                tracing::warn!(error = %e, "Failed to store canonical alias clear event");
            }
            if let Err(e) = storage
                .set_room_state(&room_id, et::CANONICAL_ALIAS, "", &event_id)
                .await
            {
                tracing::warn!(error = %e, "Failed to update canonical alias room state");
            }

            state
                .notifier()
                .notify(crate::notify::Notification::RoomEvent {
                    room_id: room_id.clone(),
                })
                .await;
        }
    }

    Ok(Json(serde_json::json!({})))
}

// -- GET /_matrix/client/v3/rooms/{roomId}/aliases --

async fn get_room_aliases(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is a member of the room
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    let aliases = storage
        .get_room_aliases(&room_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({
        "aliases": aliases,
    })))
}

// -- PUT /_matrix/client/v3/directory/list/room/{roomId} --

#[derive(Deserialize)]
struct SetVisibilityRequest {
    #[serde(default = "default_visibility")]
    visibility: String,
}

fn default_visibility() -> String {
    "private".to_string()
}

async fn set_room_visibility(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    MatrixJson(body): MatrixJson<SetVisibilityRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();

    // Verify room exists
    storage.get_room(&room_id).await.map_err(|e| match e {
        StorageError::NotFound => MatrixError::not_found("Room not found"),
        other => crate::extractors::storage_error(other),
    })?;

    storage
        .set_room_visibility(&room_id, &body.visibility)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

// -- GET /_matrix/client/v3/publicRooms --

#[derive(Deserialize, Default)]
struct PublicRoomsQuery {
    limit: Option<usize>,
    since: Option<String>,
}

#[derive(Serialize)]
struct PublicRoomEntry {
    room_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    canonical_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<String>,
    num_joined_members: usize,
    world_readable: bool,
    guest_can_join: bool,
}

#[derive(Serialize)]
struct PublicRoomsResponse {
    chunk: Vec<PublicRoomEntry>,
    total_room_count_estimate: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_batch: Option<String>,
}

async fn get_public_rooms(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PublicRoomsQuery>,
) -> Result<Json<PublicRoomsResponse>, MatrixError> {
    let storage = state.storage();
    let limit = query.limit.unwrap_or(20).min(100);

    let (rooms, total) = storage
        .get_public_rooms(limit, query.since.as_deref(), None)
        .await
        .map_err(crate::extractors::storage_error)?;

    let next_batch = if rooms.len() >= limit {
        let start = query
            .since
            .as_deref()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        Some((start + rooms.len()).to_string())
    } else {
        None
    };

    let chunk: Vec<PublicRoomEntry> = rooms
        .into_iter()
        .map(|r| PublicRoomEntry {
            room_id: r.room_id,
            name: r.name,
            topic: r.topic,
            canonical_alias: r.canonical_alias,
            avatar_url: r.avatar_url,
            num_joined_members: r.num_joined_members,
            world_readable: r.world_readable,
            guest_can_join: r.guest_can_join,
        })
        .collect();

    Ok(Json(PublicRoomsResponse {
        chunk,
        total_room_count_estimate: total,
        next_batch,
    }))
}

// -- POST /_matrix/client/v3/publicRooms --

#[derive(Deserialize, Default)]
struct SearchPublicRoomsRequest {
    limit: Option<usize>,
    since: Option<String>,
    filter: Option<PublicRoomsFilter>,
}

#[derive(Deserialize, Default)]
struct PublicRoomsFilter {
    generic_search_term: Option<String>,
}

async fn search_public_rooms(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<SearchPublicRoomsRequest>,
) -> Result<Json<PublicRoomsResponse>, MatrixError> {
    let storage = state.storage();
    let limit = body.limit.unwrap_or(20).min(100);
    let filter_term = body
        .filter
        .as_ref()
        .and_then(|f| f.generic_search_term.as_deref());

    let (rooms, total) = storage
        .get_public_rooms(limit, body.since.as_deref(), filter_term)
        .await
        .map_err(crate::extractors::storage_error)?;

    let next_batch = if rooms.len() >= limit {
        let start = body
            .since
            .as_deref()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        Some((start + rooms.len()).to_string())
    } else {
        None
    };

    let chunk: Vec<PublicRoomEntry> = rooms
        .into_iter()
        .map(|r| PublicRoomEntry {
            room_id: r.room_id,
            name: r.name,
            topic: r.topic,
            canonical_alias: r.canonical_alias,
            avatar_url: r.avatar_url,
            num_joined_members: r.num_joined_members,
            world_readable: r.world_readable,
            guest_can_join: r.guest_can_join,
        })
        .collect();

    Ok(Json(PublicRoomsResponse {
        chunk,
        total_room_count_estimate: total,
        next_batch,
    }))
}
