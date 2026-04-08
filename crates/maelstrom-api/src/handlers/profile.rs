use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::error::MatrixError;
use maelstrom_core::events::pdu::{StoredEvent, generate_event_id, timestamp_ms};
use maelstrom_core::identifiers::UserId;
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

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
            .get_state_event(&room_id, "m.room.member", user_id)
            .await
            .map(|e| e.content)
            .unwrap_or_else(|_| serde_json::json!({"membership": "join"}));

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
        let event = StoredEvent {
            event_id: event_id.clone(),
            room_id: room_id.clone(),
            sender: user_id.to_string(),
            event_type: "m.room.member".to_string(),
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
                .set_room_state(&room_id, "m.room.member", user_id, &event_id)
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
