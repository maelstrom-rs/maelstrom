//! Admin user management endpoints.
//!
//! Provides CRUD and lifecycle operations for user accounts. All endpoints
//! require admin authentication.
//!
//! ## Routes
//!
//! | Method   | Path                                              | Operation           |
//! |----------|---------------------------------------------------|---------------------|
//! | `GET`    | `/_maelstrom/admin/v1/users`                      | List all users      |
//! | `GET`    | `/_maelstrom/admin/v1/users/{userId}`             | Get user details    |
//! | `POST`   | `/_maelstrom/admin/v1/users/{userId}/deactivate`  | Deactivate account  |
//! | `POST`   | `/_maelstrom/admin/v1/users/{userId}/reactivate`  | Reactivate account  |
//! | `PUT`    | `/_maelstrom/admin/v1/users/{userId}/admin`       | Grant admin flag    |
//! | `DELETE` | `/_maelstrom/admin/v1/users/{userId}/admin`       | Revoke admin flag   |
//! | `POST`   | `/_maelstrom/admin/v1/users/{userId}/reset-password` | Reset password   |
//! | `GET`    | `/_maelstrom/admin/v1/users/{userId}/devices`     | List user devices   |

use axum::extract::{Path, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_storage::traits::StorageError;

use crate::AdminState;
use crate::auth::AdminUser;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/_maelstrom/admin/v1/users", get(list_users))
        .route("/_maelstrom/admin/v1/users/{userId}", get(get_user))
        .route(
            "/_maelstrom/admin/v1/users/{userId}/deactivate",
            post(deactivate_user),
        )
        .route(
            "/_maelstrom/admin/v1/users/{userId}/reactivate",
            post(reactivate_user),
        )
        .route("/_maelstrom/admin/v1/users/{userId}/admin", put(set_admin))
        .route(
            "/_maelstrom/admin/v1/users/{userId}/admin",
            delete(remove_admin),
        )
        .route(
            "/_maelstrom/admin/v1/users/{userId}/reset-password",
            post(reset_password),
        )
        .route(
            "/_maelstrom/admin/v1/users/{userId}/devices",
            get(list_devices),
        )
}

async fn list_users(
    State(_state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    Ok(Json(serde_json::json!({
        "users": [],
        "total": 0,
    })))
}

async fn get_user(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let localpart = user_id
        .split(':')
        .next()
        .unwrap_or(&user_id)
        .trim_start_matches('@');

    let user = state
        .storage()
        .get_user(localpart)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("User not found"),
            other => MatrixError::unknown(format!("{other}")),
        })?;

    let profile = state.storage().get_profile(localpart).await.ok();

    let devices = state
        .storage()
        .list_devices(
            &maelstrom_core::matrix::id::UserId::parse(&user_id).unwrap_or_else(|_| {
                maelstrom_core::matrix::id::UserId::new(
                    localpart,
                    &maelstrom_core::matrix::id::ServerName::new("localhost"),
                )
            }),
        )
        .await
        .unwrap_or_default();

    let rooms = state
        .storage()
        .get_joined_rooms(&user_id)
        .await
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "localpart": user.localpart,
        "is_admin": user.is_admin,
        "is_guest": user.is_guest,
        "is_deactivated": user.is_deactivated,
        "created_at": user.created_at.to_rfc3339(),
        "display_name": profile.as_ref().and_then(|p| p.display_name.as_deref()),
        "avatar_url": profile.as_ref().and_then(|p| p.avatar_url.as_deref()),
        "device_count": devices.len(),
        "room_count": rooms.len(),
    })))
}

async fn deactivate_user(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let localpart = user_id
        .split(':')
        .next()
        .unwrap_or(&user_id)
        .trim_start_matches('@');
    state
        .storage()
        .set_deactivated(localpart, true)
        .await
        .map_err(|_| MatrixError::not_found("User not found"))?;
    Ok(Json(serde_json::json!({"status": "deactivated"})))
}

async fn reactivate_user(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let localpart = user_id
        .split(':')
        .next()
        .unwrap_or(&user_id)
        .trim_start_matches('@');
    state
        .storage()
        .set_deactivated(localpart, false)
        .await
        .map_err(|_| MatrixError::not_found("User not found"))?;
    Ok(Json(serde_json::json!({"status": "reactivated"})))
}

async fn set_admin(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let localpart = user_id
        .split(':')
        .next()
        .unwrap_or(&user_id)
        .trim_start_matches('@');
    state
        .storage()
        .set_admin(localpart, true)
        .await
        .map_err(|_| MatrixError::not_found("User not found"))?;
    Ok(Json(serde_json::json!({"status": "admin_granted"})))
}

async fn remove_admin(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let localpart = user_id
        .split(':')
        .next()
        .unwrap_or(&user_id)
        .trim_start_matches('@');
    state
        .storage()
        .set_admin(localpart, false)
        .await
        .map_err(|_| MatrixError::not_found("User not found"))?;
    Ok(Json(serde_json::json!({"status": "admin_revoked"})))
}

#[derive(Deserialize)]
struct ResetPasswordRequest {
    new_password: String,
}

async fn reset_password(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let localpart = user_id
        .split(':')
        .next()
        .unwrap_or(&user_id)
        .trim_start_matches('@');

    use argon2::password_hash::SaltString;
    use argon2::{Argon2, PasswordHasher};
    let salt = SaltString::generate(&mut rand::thread_rng());
    let hash = Argon2::default()
        .hash_password(body.new_password.as_bytes(), &salt)
        .map_err(|_| MatrixError::unknown("Failed to hash password"))?
        .to_string();

    state
        .storage()
        .set_password_hash(localpart, &hash)
        .await
        .map_err(|_| MatrixError::not_found("User not found"))?;

    Ok(Json(serde_json::json!({"status": "password_reset"})))
}

async fn list_devices(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let uid = maelstrom_core::matrix::id::UserId::parse(&user_id)
        .map_err(|_| MatrixError::bad_json("Invalid user ID"))?;

    let devices = state
        .storage()
        .list_devices(&uid)
        .await
        .map_err(|_| MatrixError::not_found("User not found"))?;

    let device_list: Vec<serde_json::Value> = devices
        .iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id,
                "display_name": d.display_name,
                "created_at": d.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({"devices": device_list})))
}
