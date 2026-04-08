use axum::extract::FromRequestParts;
use http::request::Parts;

use maelstrom_core::error::MatrixError;
use maelstrom_core::identifiers::UserId;

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
