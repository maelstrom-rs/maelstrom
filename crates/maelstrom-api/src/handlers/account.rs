//! Account management -- whoami, password change, and deactivation.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 5.7 Account](https://spec.matrix.org/v1.18/client-server-api/#account)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `GET`  | `/_matrix/client/v3/account/whoami` | Token introspection |
//! | `POST` | `/_matrix/client/v3/account/deactivate` | Permanently deactivate account |
//! | `POST` | `/_matrix/client/v3/account/password` | Change password |
//!
//! # Whoami
//!
//! Returns the `user_id`, `device_id`, and `is_guest` flag associated with the
//! access token used in the request. This is the canonical way for a client to
//! confirm which account its token belongs to.
//!
//! # Password change (UIA required)
//!
//! The client must complete User-Interactive Authentication (either
//! `m.login.password` with the current password, or `m.login.dummy` for
//! passwordless accounts). On success the password hash is updated, and if
//! `logout_devices` is true (the default) all other sessions are invalidated --
//! only the device making the request survives.
//!
//! # Account deactivation (UIA required)
//!
//! Also requires UIA. Once deactivated, the user record is flagged, all devices
//! and access tokens are removed, and future login attempts are rejected with
//! `M_USER_DEACTIVATED`. Deactivation is currently irreversible.

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::handlers::util;
use crate::state::AppState;

/// Register all account management routes.
///
/// Routes:
/// - `GET  /_matrix/client/v3/account/whoami` -- identify the token's owner
/// - `POST /_matrix/client/v3/account/deactivate` -- permanently deactivate (UIA)
/// - `POST /_matrix/client/v3/account/password` -- change password (UIA)
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/account/whoami", get(whoami))
        .route("/_matrix/client/v3/account/deactivate", post(deactivate))
        .route("/_matrix/client/v3/account/password", post(change_password))
}

// -- GET /account/whoami --

/// Response for `GET /account/whoami`.
///
/// Tells the client which user and device the current access token belongs
/// to, and whether the account was registered as a guest.
#[derive(Serialize)]
struct WhoamiResponse {
    user_id: String,
    device_id: String,
    is_guest: bool,
}

async fn whoami(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<WhoamiResponse>, MatrixError> {
    let user = state
        .storage()
        .get_user(auth.user_id.localpart())
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(WhoamiResponse {
        user_id: auth.user_id.to_string(),
        device_id: auth.device_id.to_string(),
        is_guest: user.is_guest,
    }))
}

// -- POST /account/deactivate --

/// Request body for `POST /account/deactivate`.
///
/// Must include an `auth` block that completes UIA. After deactivation the
/// account cannot be reused and all devices are destroyed.
#[derive(Deserialize)]
struct DeactivateRequest {
    auth: Option<DeactivateAuth>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DeactivateAuth {
    #[serde(rename = "type")]
    auth_type: String,
    session: Option<String>,
    password: Option<String>,
}

async fn deactivate(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<DeactivateRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), MatrixError> {
    // Require UIA with password
    match &body.auth {
        Some(uia) if uia.auth_type == "m.login.password" => {
            let password = uia
                .password
                .as_deref()
                .ok_or_else(|| MatrixError::bad_json("Missing password in auth"))?;

            let user = state
                .storage()
                .get_user(auth.user_id.localpart())
                .await
                .map_err(crate::extractors::storage_error)?;

            let hash = user
                .password_hash
                .as_deref()
                .ok_or_else(|| MatrixError::forbidden("Cannot verify password"))?;

            if util::verify_password(password.to_string(), hash.to_string())
                .await
                .is_err()
            {
                // Wrong password — return 401 with UIA flows so client can retry
                let session = util::generate_session_id();
                let response = serde_json::json!({
                    "flows": [
                        { "stages": ["m.login.password"] },
                        { "stages": ["m.login.dummy"] }
                    ],
                    "session": session,
                    "errcode": "M_FORBIDDEN",
                    "error": "Invalid password"
                });
                return Ok((StatusCode::UNAUTHORIZED, Json(response)));
            }
        }
        Some(uia) if uia.auth_type == "m.login.dummy" => {
            // Allow dummy auth for accounts without passwords (e.g., guests)
        }
        _ => {
            // Return 401 with UIA flows
            let session = util::generate_session_id();
            let response = serde_json::json!({
                "flows": [
                    { "stages": ["m.login.password"] },
                    { "stages": ["m.login.dummy"] }
                ],
                "session": session
            });
            return Ok((StatusCode::UNAUTHORIZED, Json(response)));
        }
    }

    // Deactivate account
    state
        .storage()
        .set_deactivated(auth.user_id.localpart(), true)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Remove all devices
    state
        .storage()
        .remove_all_devices(&auth.user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "id_server_unbind_result": "no-support" })),
    ))
}

// -- POST /account/password --

fn default_true() -> bool {
    true
}

/// Request body for `POST /account/password`.
///
/// Requires UIA via the `auth` block. The `new_password` is Argon2-hashed
/// before storage. When `logout_devices` is true (the default), all sessions
/// except the one making the request are invalidated and associated pushers
/// are removed.
#[derive(Deserialize)]
struct ChangePasswordRequest {
    new_password: String,
    #[serde(default = "default_true")]
    logout_devices: bool,
    auth: Option<DeactivateAuth>,
}

async fn change_password(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<ChangePasswordRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), MatrixError> {
    // Require UIA
    match &body.auth {
        Some(uia) if uia.auth_type == "m.login.password" => {
            let password = uia
                .password
                .as_deref()
                .ok_or_else(|| MatrixError::bad_json("Missing password in auth"))?;

            let user = state
                .storage()
                .get_user(auth.user_id.localpart())
                .await
                .map_err(crate::extractors::storage_error)?;

            let hash = user
                .password_hash
                .as_deref()
                .ok_or_else(|| MatrixError::forbidden("Cannot verify password"))?;

            util::verify_password(password.to_string(), hash.to_string())
                .await
                .map_err(|_| MatrixError::forbidden("Invalid password"))?;
        }
        Some(uia) if uia.auth_type == "m.login.dummy" => {}
        _ => {
            let session = util::generate_session_id();
            let response = serde_json::json!({
                "flows": [
                    { "stages": ["m.login.password"] },
                    { "stages": ["m.login.dummy"] }
                ],
                "session": session
            });
            return Ok((StatusCode::UNAUTHORIZED, Json(response)));
        }
    }

    // Hash new password
    let new_hash = util::hash_password(&body.new_password)
        .await
        .map_err(MatrixError::unknown)?;

    state
        .storage()
        .set_password_hash(auth.user_id.localpart(), &new_hash)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Optionally log out all other devices (keep current session)
    if body.logout_devices {
        state
            .storage()
            .remove_all_devices_except(&auth.user_id, &auth.device_id)
            .await
            .map_err(crate::extractors::storage_error)?;

        // Remove pushers created by other sessions (keep current token's pushers)
        let user_id = auth.user_id.to_string();
        let current_token = auth.access_token.clone();
        if let Ok(pushers_data) = state
            .storage()
            .get_account_data(&user_id, None, "_maelstrom.pushers")
            .await
            && let Some(pushers) = pushers_data.get("items").and_then(|i| i.as_array())
        {
            let kept: Vec<serde_json::Value> = pushers
                .iter()
                .filter(|p| p.get("_access_token").and_then(|t| t.as_str()) == Some(&current_token))
                .cloned()
                .collect();
            let _ = state
                .storage()
                .set_account_data(
                    &user_id,
                    None,
                    "_maelstrom.pushers",
                    &serde_json::json!({"items": kept}),
                )
                .await;
        }
    }

    Ok((StatusCode::OK, Json(serde_json::json!({}))))
}
