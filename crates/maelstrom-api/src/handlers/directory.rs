use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::error::MatrixError;
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
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
                maelstrom_core::error::ErrorCode::Unknown,
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
    let storage = state.storage();

    let room_id = storage
        .get_room_alias(&room_alias)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Room alias not found"),
            other => crate::extractors::storage_error(other),
        })?;

    let server_name = state.server_name().as_str().to_string();

    Ok(Json(serde_json::json!({
        "room_id": room_id,
        "servers": [server_name],
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
            .get_state_event(&room_id, "m.room.power_levels", "")
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
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|_| MatrixError::forbidden("You are not in this room"))?;

    if membership != "join" {
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
