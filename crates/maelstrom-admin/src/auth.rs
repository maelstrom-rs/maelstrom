//! Admin authentication -- server-admin verification extractor.
//!
//! Provides the [`AdminUser`] Axum extractor, which validates incoming requests
//! in two steps:
//!
//! 1. **Token lookup** -- Extracts the `Bearer` token from the `Authorization`
//!    header and resolves it to a device record via `Storage::get_device_by_token`.
//!    This is the same mechanism the Client-Server API uses for regular users.
//!
//! 2. **Admin check** -- Loads the user account and verifies that `is_admin` is
//!    `true`. Non-admin users receive `M_FORBIDDEN`; missing or invalid tokens
//!    receive `M_MISSING_TOKEN` or `M_UNKNOWN_TOKEN`.
//!
//! The extractor yields an [`AdminUser`] containing the verified `UserId`, which
//! handlers can use for audit logging or scoped operations.

use axum::extract::FromRequestParts;
use http::request::Parts;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::UserId;

use crate::AdminState;

/// Extractor that validates the request comes from an admin user.
///
/// Checks the Bearer token against storage, then verifies is_admin flag.
pub struct AdminUser {
    pub user_id: UserId,
}

#[allow(clippy::manual_async_fn)]
impl FromRequestParts<AdminState> for AdminUser {
    type Rejection = MatrixError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AdminState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            // Extract token from Authorization header
            let token = parts
                .headers
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .ok_or_else(MatrixError::missing_token)?;

            // Look up device by token
            let device = state
                .storage()
                .get_device_by_token(token)
                .await
                .map_err(|_| MatrixError::unauthorized("Invalid access token"))?;

            // Parse user ID
            let user_id = UserId::parse(&device.user_id)
                .map_err(|_| MatrixError::unauthorized("Invalid user ID"))?;

            // Check admin flag
            let user = state
                .storage()
                .get_user(user_id.localpart())
                .await
                .map_err(|_| MatrixError::unauthorized("User not found"))?;

            if !user.is_admin {
                return Err(MatrixError::forbidden("Admin access required"));
            }

            Ok(AdminUser { user_id })
        }
    }
}
