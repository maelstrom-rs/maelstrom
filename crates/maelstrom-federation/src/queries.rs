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
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;

use crate::FederationState;

/// Build the queries sub-router with profile and directory lookup endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/federation/v1/query/profile", get(query_profile))
        .route(
            "/_matrix/federation/v1/query/directory",
            get(query_directory),
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
