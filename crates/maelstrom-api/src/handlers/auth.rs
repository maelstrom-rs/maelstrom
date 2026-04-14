//! Authentication handlers -- login, logout, and session management.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 5.5 Login](https://spec.matrix.org/v1.18/client-server-api/#login)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `GET`  | `/_matrix/client/v3/login` | Advertise supported login flows |
//! | `POST` | `/_matrix/client/v3/login` | Authenticate and obtain an access token |
//! | `POST` | `/_matrix/client/v3/logout` | Invalidate the current access token |
//! | `POST` | `/_matrix/client/v3/logout/all` | Invalidate all tokens for the user |
//!
//! # Login flow (`m.login.password`)
//!
//! 1. Client calls `GET /login` to discover that `m.login.password` is available.
//! 2. Client sends `POST /login` with an `m.id.user` identifier (or the legacy
//!    `user` field) plus a plaintext `password`.
//! 3. The server resolves the localpart (lowercased, as required by the spec),
//!    verifies the password hash with Argon2, and -- on success -- creates a new
//!    [`DeviceRecord`](maelstrom_storage::traits::DeviceRecord) and access token.
//! 4. If the client supplies a `device_id`, the server reuses it (allowing
//!    session resumption); otherwise a fresh device ID is generated.
//!
//! On successful login the server also records a device-list change position so
//! that other users sharing rooms with this user will see the new device on their
//! next `/sync`.
//!
//! # Logout
//!
//! - `POST /logout` removes only the device (and its access token) that made the
//!   request.
//! - `POST /logout/all` removes every device owned by the user, invalidating all
//!   active sessions.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::{DeviceId, UserId};
use maelstrom_core::matrix::room::account_data_type;
use maelstrom_storage::traits::{DeviceRecord, StorageError};

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::handlers::util;
use crate::state::AppState;

/// Register all authentication routes.
///
/// Routes:
/// - `GET/POST /_matrix/client/v3/login` -- login flow discovery and credential exchange
/// - `GET/POST /_matrix/client/r0/login` -- legacy r0 compatibility alias
/// - `POST /_matrix/client/v3/logout` -- single-session logout
/// - `POST /_matrix/client/v3/logout/all` -- all-session logout
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/login", get(get_login).post(post_login))
        .route("/_matrix/client/r0/login", get(get_login).post(post_login))
        .route("/_matrix/client/v3/logout", post(post_logout))
        .route("/_matrix/client/v3/logout/all", post(post_logout_all))
}

// -- GET /login --

/// Response body for `GET /login`.
///
/// Returns the list of supported authentication flows. Currently only
/// `m.login.password` is advertised.
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

/// Request body for `POST /login`.
///
/// The client must set `type` to `"m.login.password"` and provide either a
/// structured `identifier` (`{ "type": "m.id.user", "user": "alice" }`) or the
/// legacy top-level `user` field. The `password` is transmitted in cleartext
/// (the transport layer MUST use TLS). An optional `device_id` allows the
/// client to resume an existing device session instead of creating a new one.
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

/// Successful login response.
///
/// Contains the fully-qualified `user_id`, a fresh `access_token`, the
/// `device_id` (newly generated or reused from the request), and the
/// `home_server` name for client reference.
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
            maelstrom_core::matrix::error::ErrorCode::UserDeactivated,
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

    // Notify remote servers about new device via federation EDU
    if let Some(sender) = state.transaction_sender() {
        let remote_servers = crate::handlers::util::servers_sharing_rooms(
            state.storage(),
            user_id.as_ref(),
            state.server_name().as_str(),
        )
        .await;
        for server in remote_servers {
            sender.queue_edu(
                &server,
                serde_json::json!({
                    "edu_type": "m.device_list_update",
                    "content": {
                        "user_id": user_id.to_string(),
                        "device_id": device_id,
                        "stream_id": change_pos,
                        "deleted": false,
                    }
                }),
            );
        }
    }

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
    let user_id = auth.user_id.to_string();
    let device_id = auth.device_id.to_string();

    state
        .storage()
        .remove_device(&auth.user_id, &auth.device_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Clean up device-specific local notification settings (MSC3890)
    let notif_key = format!(
        "{}{}",
        account_data_type::LOCAL_NOTIFICATION_SETTINGS_PREFIX,
        device_id
    );
    if let Err(e) = state
        .storage()
        .delete_account_data(auth.user_id.as_ref(), None, &notif_key)
        .await
    {
        tracing::warn!(user = %auth.user_id, key = %notif_key, error = %e, "Failed to delete device notification settings on logout");
    }

    // Record device list change and notify remote servers
    let change_pos = state.storage().current_stream_position().await.unwrap_or(0);
    let _ = state
        .storage()
        .set_account_data(
            &user_id,
            None,
            "_maelstrom.device_change_pos",
            &serde_json::json!({"pos": change_pos}),
        )
        .await;

    if let Some(sender) = state.transaction_sender() {
        let remote_servers = crate::handlers::util::servers_sharing_rooms(
            state.storage(),
            &user_id,
            state.server_name().as_str(),
        )
        .await;
        for server in remote_servers {
            sender.queue_edu(
                &server,
                serde_json::json!({
                    "edu_type": "m.device_list_update",
                    "content": {
                        "user_id": user_id,
                        "device_id": device_id,
                        "stream_id": change_pos,
                        "deleted": true,
                    }
                }),
            );
        }
    }

    Ok(Json(serde_json::json!({})))
}

// -- POST /logout/all --

async fn post_logout_all(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    // Get device list before removal so we can clean up notification settings
    let devices = state
        .storage()
        .list_devices(&auth.user_id)
        .await
        .unwrap_or_default();

    state
        .storage()
        .remove_all_devices(&auth.user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Clean up device-specific local notification settings (MSC3890) for all devices
    for dev in &devices {
        let notif_key = format!(
            "{}{}",
            account_data_type::LOCAL_NOTIFICATION_SETTINGS_PREFIX,
            dev.device_id
        );
        let _ = state
            .storage()
            .delete_account_data(auth.user_id.as_ref(), None, &notif_key)
            .await;
    }

    // Record device list change and notify remote servers (all devices removed)
    let change_pos = state.storage().current_stream_position().await.unwrap_or(0);
    let _ = state
        .storage()
        .set_account_data(
            &user_id,
            None,
            "_maelstrom.device_change_pos",
            &serde_json::json!({"pos": change_pos}),
        )
        .await;

    if let Some(sender) = state.transaction_sender() {
        let remote_servers = crate::handlers::util::servers_sharing_rooms(
            state.storage(),
            &user_id,
            state.server_name().as_str(),
        )
        .await;
        for server in remote_servers {
            sender.queue_edu(
                &server,
                serde_json::json!({
                    "edu_type": "m.device_list_update",
                    "content": {
                        "user_id": user_id,
                        "stream_id": change_pos,
                        "deleted": true,
                    }
                }),
            );
        }
    }

    Ok(Json(serde_json::json!({})))
}
