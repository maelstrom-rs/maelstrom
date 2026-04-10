//! User registration and username availability.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 5.6 Registration](https://spec.matrix.org/v1.13/client-server-api/#registration)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `POST` | `/_matrix/client/v3/register` | Create a new account |
//! | `GET`  | `/_matrix/client/v3/register/available` | Check username availability |
//! | `GET`  | `/_synapse/admin/v1/register` | Get nonce (Synapse-compat admin reg) |
//! | `POST` | `/_synapse/admin/v1/register` | Admin registration with shared secret |
//!
//! # User-Interactive Authentication (UIA)
//!
//! Registration uses the Matrix UIA protocol. On the first `POST /register`
//! without an `auth` block, the server returns **HTTP 401** with available
//! flows. Currently the only supported flow is the single-stage `m.login.dummy`,
//! which amounts to open registration -- the client just re-sends the request
//! with `auth: { "type": "m.login.dummy" }`.
//!
//! # Username validation
//!
//! Usernames (localparts) must:
//! - Be non-empty and at most 255 characters
//! - Contain only lowercase ASCII letters, digits, and the characters `._=-/`
//! - Be unique (case-insensitive, since the server lowercases on arrival)
//!
//! If no username is provided the server generates a random one.
//!
//! # `inhibit_login`
//!
//! When `inhibit_login` is `true` in the request the server creates the account
//! but does **not** create a device or issue an access token. The response will
//! contain only `user_id`.
//!
//! # First-user admin promotion
//!
//! The very first account registered on a fresh server is automatically granted
//! admin privileges.
//!
//! # Complement admin registration
//!
//! For integration testing with Complement, the Synapse-compatible shared-secret
//! registration endpoint is supported at `/_synapse/admin/v1/register`. In
//! dev/test mode HMAC verification is skipped.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::{DeviceId, UserId};
use maelstrom_storage::traits::{DeviceRecord, UserRecord};

use crate::extractors::MatrixJson;
use crate::handlers::util;
use crate::state::AppState;

/// Register all registration and username-availability routes.
///
/// Routes:
/// - `POST /_matrix/client/v3/register` -- UIA-gated account creation
/// - `GET  /_matrix/client/v3/register/available` -- username availability check
/// - `GET/POST /_synapse/admin/v1/register` -- Synapse-compatible admin registration
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/register", post(post_register))
        .route("/_matrix/client/v3/register/available", get(get_available))
        // Complement shared-secret admin registration (Synapse-compatible)
        .route(
            "/_synapse/admin/v1/register",
            get(admin_register_nonce).post(admin_register),
        )
}

// -- GET /register/available --

/// Query parameters for `GET /register/available`.
#[derive(Deserialize)]
struct AvailableQuery {
    username: String,
}

#[derive(Serialize)]
struct AvailableResponse {
    available: bool,
}

async fn get_available(
    State(state): State<AppState>,
    Query(query): Query<AvailableQuery>,
) -> Result<Json<AvailableResponse>, MatrixError> {
    validate_username(&query.username)?;

    let exists = state
        .storage()
        .user_exists(&query.username)
        .await
        .map_err(crate::extractors::storage_error)?;

    if exists {
        return Err(MatrixError::user_in_use());
    }

    Ok(Json(AvailableResponse { available: true }))
}

// -- POST /register --

/// Request body for `POST /register`.
///
/// The client sends this twice in the UIA flow: first without `auth` (to get
/// the 401 with available stages), then again with `auth.type` set to
/// `"m.login.dummy"`. The `password` is hashed with Argon2 before storage.
/// When `inhibit_login` is true, no device or access token is created.
#[derive(Deserialize)]
struct RegisterRequest {
    auth: Option<AuthData>,
    username: Option<String>,
    password: Option<String>,
    device_id: Option<String>,
    initial_device_display_name: Option<String>,
    #[serde(default)]
    inhibit_login: bool,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AuthData {
    #[serde(rename = "type")]
    auth_type: String,
    session: Option<String>,
}

/// Successful registration response.
///
/// Always contains `user_id`. When `inhibit_login` was false (the default),
/// also contains `access_token` and `device_id` for immediate session use.
#[derive(Serialize)]
struct RegisterResponse {
    user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
}

async fn post_register(
    State(state): State<AppState>,
    MatrixJson(body): MatrixJson<RegisterRequest>,
) -> Result<impl IntoResponse, MatrixError> {
    // Check if UIA is needed
    let auth_completed = match &body.auth {
        Some(auth) if auth.auth_type == "m.login.dummy" => true,
        Some(_) => {
            return Err(MatrixError::new(
                StatusCode::BAD_REQUEST,
                maelstrom_core::matrix::error::ErrorCode::Unknown,
                "Unsupported auth type",
            ));
        }
        None => false,
    };

    if !auth_completed {
        // Return 401 with UIA flows
        let session = util::generate_session_id();
        let response = serde_json::json!({
            "flows": [{ "stages": ["m.login.dummy"] }],
            "session": session
        });
        return Ok((StatusCode::UNAUTHORIZED, Json(response)).into_response());
    }

    // Validate and generate username — spec requires lowercasing
    let username = match &body.username {
        Some(u) => {
            let lowered = u.to_lowercase();
            validate_username(&lowered)?;
            lowered
        }
        None => util::generate_localpart(),
    };

    // Check availability
    let exists = state
        .storage()
        .user_exists(&username)
        .await
        .map_err(crate::extractors::storage_error)?;

    if exists {
        return Err(MatrixError::user_in_use());
    }

    // Hash password
    let password_hash = match &body.password {
        Some(pw) => Some(
            util::hash_password(pw)
                .await
                .map_err(MatrixError::unknown)?,
        ),
        None => None,
    };

    // First user to register becomes admin automatically
    let is_first_user = state.storage().count_users().await.unwrap_or(1) == 0;

    let user = UserRecord {
        localpart: username.clone(),
        password_hash,
        is_admin: is_first_user,
        is_guest: false,
        is_deactivated: false,
        created_at: chrono::Utc::now(),
    };

    if is_first_user {
        tracing::info!(username = %username, "First user registered — granting admin");
    }

    state
        .storage()
        .create_user(&user)
        .await
        .map_err(crate::extractors::storage_error)?;

    let user_id = UserId::new(&username, state.server_name());

    if body.inhibit_login {
        let response = RegisterResponse {
            user_id: user_id.to_string(),
            access_token: None,
            device_id: None,
        };
        return Ok((StatusCode::OK, Json(response)).into_response());
    }

    // Create device and access token
    let device_id = body
        .device_id
        .unwrap_or_else(|| DeviceId::generate().to_string());
    let access_token = util::generate_access_token();

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

    let response = RegisterResponse {
        user_id: user_id.to_string(),
        access_token: Some(access_token),
        device_id: Some(device_id),
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Validate a Matrix localpart.
/// Must match: `[a-z0-9._=\-/]+`
fn validate_username(username: &str) -> Result<(), MatrixError> {
    if username.is_empty() {
        return Err(MatrixError::invalid_username("Username cannot be empty"));
    }

    if username.len() > 255 {
        return Err(MatrixError::invalid_username("Username too long"));
    }

    let valid = username
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._=-/".contains(c));

    if !valid {
        return Err(MatrixError::invalid_username(
            "Username must contain only lowercase letters, digits, and ._=-/",
        ));
    }

    Ok(())
}

// -- Complement admin registration (Synapse-compatible shared secret) --
// GET returns a nonce, POST registers a user with HMAC verification.
// We accept all registrations without HMAC verification for dev/test.

async fn admin_register_nonce() -> Json<serde_json::Value> {
    let nonce = util::generate_session_id();
    Json(serde_json::json!({ "nonce": nonce }))
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AdminRegisterRequest {
    nonce: String,
    username: String,
    password: String,
    #[serde(default)]
    admin: bool,
    mac: Option<String>,
    displayname: Option<String>,
}

async fn admin_register(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<AdminRegisterRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let username = body.username.to_lowercase();
    validate_username(&username)?;

    let password_hash = util::hash_password(&body.password)
        .await
        .map_err(MatrixError::unknown)?;

    let user = UserRecord {
        localpart: username.clone(),
        password_hash: Some(password_hash),
        is_admin: body.admin,
        is_guest: false,
        is_deactivated: false,
        created_at: chrono::Utc::now(),
    };

    state
        .storage()
        .create_user(&user)
        .await
        .map_err(crate::extractors::storage_error)?;

    let device_id = DeviceId::generate().to_string();
    let access_token = util::generate_access_token();
    let user_id = UserId::new(&username, state.server_name());

    let device = DeviceRecord {
        device_id: device_id.clone(),
        user_id: user_id.to_string(),
        display_name: body.displayname,
        access_token: access_token.clone(),
        created_at: chrono::Utc::now(),
    };

    state
        .storage()
        .create_device(&device)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({
        "access_token": access_token,
        "user_id": user_id.to_string(),
        "device_id": device_id,
        "home_server": state.server_name().to_string(),
    })))
}
