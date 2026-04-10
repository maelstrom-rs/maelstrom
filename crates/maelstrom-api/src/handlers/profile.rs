//! User profile management -- display names, avatars, and user directory search.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 7.1 Profiles](https://spec.matrix.org/v1.13/client-server-api/#profiles)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `GET`  | `/_matrix/client/v3/profile/{userId}/displayname` | Get display name |
//! | `PUT`  | `/_matrix/client/v3/profile/{userId}/displayname` | Set display name |
//! | `GET`  | `/_matrix/client/v3/profile/{userId}/avatar_url` | Get avatar URL |
//! | `PUT`  | `/_matrix/client/v3/profile/{userId}/avatar_url` | Set avatar URL |
//! | `GET`  | `/_matrix/client/v3/profile/{userId}` | Get full profile |
//! | `POST` | `/_matrix/client/v3/user_directory/search` | Search users by name |
//!
//! # Profile model
//!
//! Profiles are global and per-user -- **not** per-room. A single display name
//! and avatar URL are stored for each user. Any authenticated user can read any
//! other user's profile, but writes are restricted to the profile owner (enforced
//! by [`verify_profile_owner`]).
//!
//! # Room propagation
//!
//! When a user updates their display name or avatar, the server automatically
//! re-emits `m.room.member` state events in every room the user has joined.
//! This ensures that sync responses for other room members reflect the updated
//! profile. The propagation is handled by [`propagate_profile_to_rooms`], which
//! skips rooms where the content would not actually change.
//!
//! # User directory search
//!
//! `POST /user_directory/search` performs a case-insensitive prefix search over
//! all registered users and returns up to 50 results with their display names
//! and avatar URLs.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::{Pdu, generate_event_id, timestamp_ms};
use maelstrom_core::matrix::id::UserId;
use maelstrom_core::matrix::room::{Membership, event_type as et};
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

/// Register all profile and user directory routes.
///
/// Routes:
/// - `GET/PUT /_matrix/client/v3/profile/{userId}/displayname`
/// - `GET/PUT /_matrix/client/v3/profile/{userId}/avatar_url`
/// - `GET     /_matrix/client/v3/profile/{userId}` -- full profile
/// - `POST    /_matrix/client/v3/user_directory/search`
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/_matrix/client/v3/profile/{userId}/displayname",
            get(get_displayname).put(put_displayname),
        )
        .route(
            "/_matrix/client/v3/profile/{userId}/avatar_url",
            get(get_avatar_url).put(put_avatar_url),
        )
        .route("/_matrix/client/v3/profile/{userId}", get(get_profile))
        .route(
            "/_matrix/client/v3/user_directory/search",
            post(search_user_directory),
        )
}

// -- GET /profile/{userId}/displayname --

#[derive(Serialize)]
struct DisplayNameResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    displayname: Option<String>,
}

async fn get_displayname(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<DisplayNameResponse>, MatrixError> {
    let localpart = extract_localpart(&user_id)?;

    let profile = state
        .storage()
        .get_profile(&localpart)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("User not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(DisplayNameResponse {
        displayname: profile.display_name,
    }))
}

// -- PUT /profile/{userId}/displayname --

#[derive(Deserialize)]
struct SetDisplayNameRequest {
    displayname: Option<String>,
}

async fn put_displayname(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(user_id): Path<String>,
    MatrixJson(body): MatrixJson<SetDisplayNameRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    verify_profile_owner(&auth, &user_id)?;

    state
        .storage()
        .set_display_name(auth.user_id.localpart(), body.displayname.as_deref())
        .await
        .map_err(crate::extractors::storage_error)?;

    // Emit updated m.room.member events in all joined rooms
    propagate_profile_to_rooms(&state, auth.user_id.as_ref()).await;

    Ok(Json(serde_json::json!({})))
}

// -- GET /profile/{userId}/avatar_url --

#[derive(Serialize)]
struct AvatarUrlResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<String>,
}

async fn get_avatar_url(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<AvatarUrlResponse>, MatrixError> {
    let localpart = extract_localpart(&user_id)?;

    let profile = state
        .storage()
        .get_profile(&localpart)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("User not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(AvatarUrlResponse {
        avatar_url: profile.avatar_url,
    }))
}

// -- PUT /profile/{userId}/avatar_url --

#[derive(Deserialize)]
struct SetAvatarUrlRequest {
    avatar_url: Option<String>,
}

async fn put_avatar_url(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(user_id): Path<String>,
    MatrixJson(body): MatrixJson<SetAvatarUrlRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    verify_profile_owner(&auth, &user_id)?;

    state
        .storage()
        .set_avatar_url(auth.user_id.localpart(), body.avatar_url.as_deref())
        .await
        .map_err(crate::extractors::storage_error)?;

    // Emit updated m.room.member events in all joined rooms
    propagate_profile_to_rooms(&state, auth.user_id.as_ref()).await;

    Ok(Json(serde_json::json!({})))
}

// -- GET /profile/{userId} --

#[derive(Serialize)]
struct FullProfileResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    displayname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<String>,
}

async fn get_profile(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<FullProfileResponse>, MatrixError> {
    let localpart = extract_localpart(&user_id)?;

    let profile = state
        .storage()
        .get_profile(&localpart)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("User not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(FullProfileResponse {
        displayname: profile.display_name,
        avatar_url: profile.avatar_url,
    }))
}

// -- POST /user_directory/search --

#[derive(Deserialize)]
struct UserDirectorySearchRequest {
    search_term: String,
    #[serde(default = "default_search_limit")]
    limit: usize,
}

fn default_search_limit() -> usize {
    10
}

async fn search_user_directory(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<UserDirectorySearchRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let limit = body.limit.min(50);
    let server_name = state.server_name();

    let results = state
        .storage()
        .search_users(&body.search_term, limit)
        .await
        .map_err(crate::extractors::storage_error)?;

    let results: Vec<serde_json::Value> = results
        .into_iter()
        .map(|(localpart, display_name, avatar_url)| {
            let mut entry = serde_json::json!({
                "user_id": format!("@{localpart}:{server_name}"),
            });
            if let Some(name) = display_name {
                entry["display_name"] = serde_json::Value::String(name);
            }
            if let Some(url) = avatar_url {
                entry["avatar_url"] = serde_json::Value::String(url);
            }
            entry
        })
        .collect();

    Ok(Json(serde_json::json!({
        "results": results,
        "limited": false,
    })))
}

/// Extract localpart from a user ID string (could be `@alice:server` or `alice`).
fn extract_localpart(user_id: &str) -> Result<String, MatrixError> {
    if user_id.starts_with('@') {
        UserId::parse(user_id)
            .map(|u| u.localpart().to_string())
            .map_err(|_| MatrixError::not_found("Invalid user ID"))
    } else {
        Ok(user_id.to_string())
    }
}

/// Verify the authenticated user is modifying their own profile.
fn verify_profile_owner(auth: &AuthenticatedUser, target_user_id: &str) -> Result<(), MatrixError> {
    let target_localpart = extract_localpart(target_user_id)?;
    if auth.user_id.localpart() != target_localpart {
        return Err(MatrixError::forbidden(
            "Cannot modify another user's profile",
        ));
    }
    Ok(())
}

/// After a profile change, emit new m.room.member state events in every joined room
/// so that the updated displayname/avatar_url appears in sync responses.
async fn propagate_profile_to_rooms(state: &AppState, user_id: &str) {
    let storage = state.storage();

    let rooms = match storage.get_joined_rooms(user_id).await {
        Ok(r) => r,
        Err(_) => return,
    };

    // Fetch updated profile
    let localpart = match extract_localpart(user_id) {
        Ok(l) => l,
        Err(_) => return,
    };
    let profile = storage.get_profile(&localpart).await.ok();

    for room_id in rooms {
        // Get current m.room.member state to preserve membership and other fields
        let existing_content = storage
            .get_state_event(&room_id, et::MEMBER, user_id)
            .await
            .map(|e| e.content)
            .unwrap_or_else(|_| serde_json::json!({"membership": Membership::Join.as_str()}));

        // Build updated content with new profile fields
        let mut content = existing_content.clone();
        if let Some(obj) = content.as_object_mut() {
            match profile.as_ref().and_then(|p| p.display_name.as_deref()) {
                Some(name) => {
                    obj.insert(
                        "displayname".to_string(),
                        serde_json::Value::String(name.to_string()),
                    );
                }
                None => {
                    obj.remove("displayname");
                }
            }
            match profile.as_ref().and_then(|p| p.avatar_url.as_deref()) {
                Some(url) => {
                    obj.insert(
                        "avatar_url".to_string(),
                        serde_json::Value::String(url.to_string()),
                    );
                }
                None => {
                    obj.remove("avatar_url");
                }
            }
        }

        // Skip if content hasn't actually changed
        if content == existing_content {
            continue;
        }

        let event_id = generate_event_id();
        let event = Pdu {
            event_id: event_id.clone(),
            room_id: room_id.clone(),
            sender: user_id.to_string(),
            event_type: et::MEMBER.to_string(),
            state_key: Some(user_id.to_string()),
            content,
            origin_server_ts: timestamp_ms(),
            unsigned: None,
            stream_position: 0,
            origin: None,
            auth_events: None,
            prev_events: None,
            depth: None,
            hashes: None,
            signatures: None,
        };

        if storage.store_event(&event).await.is_ok() {
            let _ = storage
                .set_room_state(&room_id, et::MEMBER, user_id, &event_id)
                .await;

            state
                .notifier()
                .notify(Notification::RoomEvent {
                    room_id: room_id.clone(),
                })
                .await;
        }
    }
}
