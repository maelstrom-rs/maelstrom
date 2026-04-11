//! Access-token authentication extractor for Matrix endpoints.
//!
//! This module implements the authentication gate for the Client-Server API.
//! In Matrix, clients authenticate by sending an access token obtained during
//! login or registration.  The token can appear in two places (the spec
//! requires servers to check both):
//!
//! 1. The `Authorization: Bearer <token>` HTTP header (preferred).
//! 2. The `access_token=<token>` query parameter (legacy, but still used by
//!    some clients and for browser-based requests like media downloads).
//!
//! The extractor looks up the token in storage to resolve the associated user
//! and device.  If the token is missing or invalid, the handler never runs --
//! Axum returns a `401 M_UNKNOWN_TOKEN` or `401 M_MISSING_TOKEN` error
//! directly.

use axum::extract::FromRequestParts;
use http::request::Parts;
use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::{DeviceId, UserId};

use crate::state::AppState;

/// The authentication gate for Matrix Client-Server API endpoints.
///
/// Including `AuthenticatedUser` in a handler's parameter list is all you need
/// to require a valid access token.  Axum calls `from_request_parts` before
/// your handler runs, so by the time your handler executes, you are guaranteed
/// to have a valid `user_id` and `device_id`.
///
/// # Token resolution
///
/// 1. Checks the `Authorization: Bearer <token>` header first.
/// 2. Falls back to the `access_token` query parameter.
/// 3. Looks up the token in the device store to resolve the owning user and device.
///
/// # Example
///
/// ```rust,ignore
/// async fn get_profile(
///     State(state): State<AppState>,
///     user: AuthenticatedUser,  // <-- this line enforces auth
/// ) -> Result<Json<ProfileResponse>, MatrixError> {
///     // user.user_id and user.device_id are valid here
/// }
/// ```
pub struct AuthenticatedUser {
    /// The fully-qualified Matrix user ID (e.g. `@alice:example.com`).
    pub user_id: UserId,
    /// The device ID that owns this access token.
    pub device_id: DeviceId,
    /// The raw access token string (useful for token revocation).
    pub access_token: String,
}

impl AuthenticatedUser {
    fn extract_token(parts: &Parts) -> Result<String, MatrixError> {
        // Try Authorization header first
        if let Some(auth_header) = parts.headers.get("authorization") {
            let header_str = auth_header
                .to_str()
                .map_err(|_| MatrixError::unauthorized("Invalid Authorization header"))?;

            if let Some(token) = header_str.strip_prefix("Bearer ") {
                return Ok(token.to_string());
            }
        }

        // Fall back to query parameter
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&') {
                if let Some(token) = pair.strip_prefix("access_token=") {
                    return Ok(token.to_string());
                }
            }
        }

        Err(MatrixError::missing_token())
    }
}

impl AuthenticatedUser {
    /// Parse query parameters from the URI into a simple map.
    fn query_params(parts: &Parts) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    map.insert(key.to_string(), value.to_string());
                }
            }
        }
        map
    }
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = MatrixError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = Self::extract_token(parts)?;

        // Try normal device token lookup first
        match state.storage().get_device_by_token(&token).await {
            Ok(device) => {
                // The device store may return a full user_id (@user:server) or just a localpart,
                // depending on the backend. Handle both cases.
                let user_id = if device.user_id.starts_with('@') {
                    UserId::parse(&device.user_id)
                        .map_err(|_| MatrixError::unknown("Invalid user_id in device record"))?
                } else {
                    UserId::new(&device.user_id, state.server_name())
                };

                Ok(AuthenticatedUser {
                    user_id,
                    device_id: DeviceId::new(device.device_id),
                    access_token: token,
                })
            }
            Err(_) => {
                // If normal device token lookup fails, check if it's an AS token
                let appservice = state
                    .storage()
                    .get_appservice_by_token(&token)
                    .await
                    .map_err(|_| {
                        tracing::warn!("Token lookup failed for both device and appservice");
                        MatrixError::unauthorized("Unknown or expired access token")
                    })?;

                let query_params = Self::query_params(parts);

                // AS authenticated -- check for user impersonation
                let user_id = if let Some(impersonate) = query_params.get("user_id") {
                    // Verify the impersonated user is in the AS's namespace
                    UserId::parse(impersonate)
                        .map_err(|_| MatrixError::forbidden("Invalid user_id"))?
                } else {
                    // Default to AS's sender user
                    UserId::new(&appservice.sender_localpart, state.server_name())
                };

                Ok(AuthenticatedUser {
                    user_id,
                    device_id: DeviceId::new("appservice"),
                    access_token: token,
                })
            }
        }
    }
}
