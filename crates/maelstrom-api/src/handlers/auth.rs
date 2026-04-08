use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::error::MatrixError;
use maelstrom_core::identifiers::{DeviceId, UserId};
use maelstrom_storage::traits::{DeviceRecord, StorageError};

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::handlers::util;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/login", get(get_login).post(post_login))
        .route("/_matrix/client/r0/login", get(get_login).post(post_login))
        .route("/_matrix/client/v3/logout", post(post_logout))
        .route("/_matrix/client/v3/logout/all", post(post_logout_all))
}

// -- GET /login --

#[derive(Serialize)]
struct LoginFlowsResponse {
    flows: Vec<LoginFlow>,
}

#[derive(Serialize)]
struct LoginFlow {
    #[serde(rename = "type")]
    flow_type: &'static str,
}

async fn get_login() -> Json<LoginFlowsResponse> {
    Json(LoginFlowsResponse {
        flows: vec![LoginFlow {
            flow_type: "m.login.password",
        }],
    })
}

// -- POST /login --

#[derive(Deserialize)]
struct LoginRequest {
    #[serde(rename = "type")]
    login_type: String,
    identifier: Option<UserIdentifier>,
    // Legacy field — some clients send `user` directly
    user: Option<String>,
    password: Option<String>,
    device_id: Option<String>,
    initial_device_display_name: Option<String>,
}

#[derive(Deserialize)]
struct UserIdentifier {
    #[serde(rename = "type")]
    id_type: String,
    user: Option<String>,
}

#[derive(Serialize)]
struct LoginResponse {
    user_id: String,
    access_token: String,
    device_id: String,
    home_server: String,
}

async fn post_login(
    State(state): State<AppState>,
    MatrixJson(body): MatrixJson<LoginRequest>,
) -> Result<Json<LoginResponse>, MatrixError> {
    if body.login_type != "m.login.password" {
        return Err(MatrixError::unknown("Unsupported login type"));
    }

    let password = body
        .password
        .as_deref()
        .ok_or_else(|| MatrixError::bad_json("Missing password field"))?;

    // Resolve the username from identifier or legacy user field
    let raw_user = body
        .identifier
        .and_then(|id| {
            if id.id_type == "m.id.user" {
                id.user
            } else {
                None
            }
        })
        .or(body.user)
        .ok_or_else(|| MatrixError::bad_json("Missing user identifier"))?;

    // Extract localpart — input could be `@alice:server` or just `alice`
    // Spec requires case-insensitive matching (lowercase)
    let localpart = if raw_user.starts_with('@') {
        UserId::parse(&raw_user)
            .map(|u| u.localpart().to_lowercase())
            .map_err(|_| MatrixError::bad_json("Invalid user ID format"))?
    } else {
        raw_user.to_lowercase()
    };

    // Look up user — discriminate between not-found and actual errors
    let user = state
        .storage()
        .get_user(&localpart)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("Invalid username or password"),
            other => crate::extractors::storage_error(other),
        })?;

    if user.is_deactivated {
        return Err(MatrixError::new(
            http::StatusCode::FORBIDDEN,
            maelstrom_core::error::ErrorCode::UserDeactivated,
            "This account has been deactivated",
        ));
    }

    // Verify password
    let hash = user
        .password_hash
        .as_deref()
        .ok_or_else(|| MatrixError::forbidden("Invalid username or password"))?;

    util::verify_password(password.to_string(), hash.to_string())
        .await
        .map_err(|_| MatrixError::forbidden("Invalid username or password"))?;

    // Create device and access token
    let device_id = body
        .device_id
        .unwrap_or_else(|| DeviceId::generate().to_string());
    let access_token = util::generate_access_token();
    let user_id = UserId::new(&localpart, state.server_name());

    let device = DeviceRecord {
        device_id: device_id.clone(),
        user_id: user_id.to_string(),
        display_name: body.initial_device_display_name,
        access_token: access_token.clone(),
        created_at: chrono::Utc::now(),
    };

    state
        .storage()
        .create_device(&device)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Record device list change for sync tracking
    // Increment stream position so other users' syncs see a change
    let change_pos = state.storage().next_stream_position().await.unwrap_or(0);
    let _ = state
        .storage()
        .set_account_data(
            user_id.as_ref(),
            None,
            "_maelstrom.device_change_pos",
            &serde_json::json!({"pos": change_pos}),
        )
        .await;

    // Notify all rooms the user is in so other users' syncs wake up
    if let Ok(rooms) = state.storage().get_joined_rooms(user_id.as_ref()).await {
        for room_id in rooms {
            state
                .notifier()
                .notify(crate::notify::Notification::RoomEvent { room_id })
                .await;
        }
    }

    Ok(Json(LoginResponse {
        user_id: user_id.to_string(),
        access_token,
        device_id,
        home_server: state.server_name().to_string(),
    }))
}

// -- POST /logout --

async fn post_logout(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    state
        .storage()
        .remove_device(&auth.user_id, &auth.device_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

// -- POST /logout/all --

async fn post_logout_all(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    state
        .storage()
        .remove_all_devices(&auth.user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}
