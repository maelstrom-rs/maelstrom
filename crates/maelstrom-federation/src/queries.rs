//! # Federation Query Endpoints
//!
//! These endpoints allow remote servers to look up information about users and rooms
//! that live on this server. This is how cross-server profile resolution and room
//! alias lookups work.
//!
//! ## Profile Queries
//!
//! `GET /_matrix/federation/v1/query/profile?user_id=@alice:example.com`
//!
//! When a remote server needs to display a user's profile (display name, avatar) in
//! a room, it queries the user's home server. The `field` parameter can optionally
//! limit the response to just `displayname` or `avatar_url`.
//!
//! The endpoint validates that the requested user actually belongs to this server
//! (by checking the server part of the user ID) before returning profile data.
//!
//! ## Directory Queries
//!
//! `GET /_matrix/federation/v1/query/directory?room_alias=#room:example.com`
//!
//! When a user tries to join a room by alias (e.g., `#matrix:example.com`) and the
//! alias points to a remote server, the local server queries the remote server to
//! resolve the alias to a room ID. The response includes the room ID and a list of
//! servers that can be used to join the room.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;

use crate::FederationState;

/// Build the queries sub-router with profile, directory, and public rooms endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/federation/v1/query/profile", get(query_profile))
        .route(
            "/_matrix/federation/v1/query/directory",
            get(query_directory),
        )
        .route(
            "/_matrix/federation/v1/publicRooms",
            get(get_public_rooms_fed).post(search_public_rooms_fed),
        )
}

/// Query parameters for the profile lookup endpoint.
#[derive(Deserialize)]
struct ProfileQuery {
    /// The full Matrix user ID (e.g., `@alice:example.com`).
    user_id: String,
    /// Optional field filter: `"displayname"` or `"avatar_url"`. When omitted,
    /// both fields are returned.
    field: Option<String>,
}

/// GET /_matrix/federation/v1/query/profile — serve local user profile to remote servers.
async fn query_profile(
    State(state): State<FederationState>,
    Query(query): Query<ProfileQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(user_id = %query.user_id, field = ?query.field, "Federation profile query");

    // Validate user_id format using proper parser
    let parsed = maelstrom_core::matrix::id::UserId::parse(&query.user_id).map_err(|_| {
        MatrixError::new(
            http::StatusCode::BAD_REQUEST,
            maelstrom_core::matrix::error::ErrorCode::InvalidParam,
            "Invalid user_id",
        )
    })?;

    // Also validate the server_name portion is well-formed
    maelstrom_core::matrix::id::validate_server_name(parsed.server_name()).map_err(|_| {
        MatrixError::new(
            http::StatusCode::BAD_REQUEST,
            maelstrom_core::matrix::error::ErrorCode::InvalidParam,
            format!("Invalid server name in user_id: {}", parsed.server_name()),
        )
    })?;

    if parsed.server_name() != state.server_name().as_str() {
        return Err(MatrixError::not_found("User not on this server"));
    }

    let localpart = parsed.localpart();

    // Check user exists
    if !state
        .storage()
        .user_exists(localpart)
        .await
        .unwrap_or(false)
    {
        return Err(MatrixError::not_found("User not found"));
    }

    let profile = state
        .storage()
        .get_profile(localpart)
        .await
        .map_err(|_| MatrixError::not_found("Profile not found"))?;

    match query.field.as_deref() {
        Some("displayname") => Ok(Json(serde_json::json!({
            "displayname": profile.display_name,
        }))),
        Some("avatar_url") => Ok(Json(serde_json::json!({
            "avatar_url": profile.avatar_url,
        }))),
        _ => {
            let mut resp = serde_json::json!({});
            if let Some(name) = &profile.display_name {
                resp["displayname"] = serde_json::json!(name);
            }
            if let Some(url) = &profile.avatar_url {
                resp["avatar_url"] = serde_json::json!(url);
            }
            Ok(Json(resp))
        }
    }
}

/// Query parameters for the directory (room alias) lookup endpoint.
#[derive(Deserialize)]
struct DirectoryQuery {
    /// The room alias to resolve (e.g., `#room:example.com`).
    room_alias: String,
}

/// GET /_matrix/federation/v1/query/directory — resolve a room alias for remote servers.
async fn query_directory(
    State(state): State<FederationState>,
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_alias = %query.room_alias, "Federation directory query");

    let room_id = state
        .storage()
        .get_room_alias(&query.room_alias)
        .await
        .map_err(|_| MatrixError::not_found("Room alias not found"))?;

    Ok(Json(serde_json::json!({
        "room_id": room_id,
        "servers": [state.server_name().as_str()],
    })))
}

// ---------------------------------------------------------------------------
// Federation Public Rooms (spec: Server-Server API § 11.1)
// ---------------------------------------------------------------------------

/// Query parameters for `GET /_matrix/federation/v1/publicRooms`.
#[derive(Deserialize, Default)]
struct PublicRoomsQuery {
    limit: Option<usize>,
    since: Option<String>,
}

/// A single entry in the public room directory response.
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

/// The public rooms directory response (shared by GET and POST).
#[derive(Serialize)]
struct PublicRoomsResponse {
    chunk: Vec<PublicRoomEntry>,
    total_room_count_estimate: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_batch: Option<String>,
}

/// GET /_matrix/federation/v1/publicRooms — list public rooms for remote servers.
async fn get_public_rooms_fed(
    State(state): State<FederationState>,
    Query(query): Query<PublicRoomsQuery>,
) -> Result<Json<PublicRoomsResponse>, MatrixError> {
    debug!("Federation publicRooms GET");

    let limit = query.limit.unwrap_or(20).min(100);

    let (rooms, total) = state
        .storage()
        .get_public_rooms(limit, query.since.as_deref(), None)
        .await
        .map_err(|_| MatrixError::unknown("Failed to fetch public rooms"))?;

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

    let chunk = rooms
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

/// Request body for `POST /_matrix/federation/v1/publicRooms`.
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

/// POST /_matrix/federation/v1/publicRooms — search/filter public rooms for remote servers.
async fn search_public_rooms_fed(
    State(state): State<FederationState>,
    Json(body): Json<SearchPublicRoomsRequest>,
) -> Result<Json<PublicRoomsResponse>, MatrixError> {
    debug!("Federation publicRooms POST (search)");

    let limit = body.limit.unwrap_or(20).min(100);
    let filter_term = body
        .filter
        .as_ref()
        .and_then(|f| f.generic_search_term.as_deref());

    let (rooms, total) = state
        .storage()
        .get_public_rooms(limit, body.since.as_deref(), filter_term)
        .await
        .map_err(|_| MatrixError::unknown("Failed to fetch public rooms"))?;

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

    let chunk = rooms
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
