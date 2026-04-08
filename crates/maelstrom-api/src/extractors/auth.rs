use axum::extract::FromRequestParts;
use http::request::Parts;
use maelstrom_core::error::MatrixError;
use maelstrom_core::identifiers::{DeviceId, UserId};

use crate::state::AppState;

/// Extractor that validates an access token and provides authenticated user info.
///
/// Extracts the access token from either:
/// - `Authorization: Bearer <token>` header
/// - `access_token=<token>` query parameter
///
/// Looks up the token in storage and resolves the associated user and device.
pub struct AuthenticatedUser {
    pub user_id: UserId,
    pub device_id: DeviceId,
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

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = MatrixError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = Self::extract_token(parts)?;

        let device = state
            .storage()
            .get_device_by_token(&token)
            .await
            .map_err(|e| {
                tracing::warn!("Token lookup failed: {e}");
                MatrixError::unauthorized("Unknown or expired access token")
            })?;

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
}
