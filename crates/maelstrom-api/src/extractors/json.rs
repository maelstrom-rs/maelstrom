//! Matrix-spec-compliant JSON body extraction with proper error codes.
//!
//! Axum's built-in `Json<T>` extractor returns plain-text error messages when
//! deserialization fails.  The Matrix spec requires errors to be JSON objects
//! with `errcode` and `error` fields.  This module provides [`MatrixJson`],
//! a drop-in replacement that returns the correct error format.

use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use maelstrom_core::matrix::error::MatrixError;
use serde::de::DeserializeOwned;

/// Drop-in replacement for `axum::Json<T>` that returns Matrix-spec-compliant
/// error responses.
///
/// # Error mapping
///
/// | Condition | Matrix error code |
/// |---|---|
/// | `Content-Type` is not `application/json` | `M_NOT_JSON` |
/// | Body is not valid UTF-8 | `M_NOT_JSON` |
/// | Body fails to deserialize into `T` | `M_BAD_JSON` |
/// | Body exceeds 1 MiB | `M_BAD_JSON` |
///
/// # Usage
///
/// Use `MatrixJson` in place of `Json` for any handler that accepts a JSON
/// request body:
///
/// ```rust,ignore
/// async fn send_message(
///     MatrixJson(body): MatrixJson<SendMessageRequest>,
/// ) -> Result<Json<SendMessageResponse>, MatrixError> { ... }
/// ```
///
/// `MatrixJson` also implements `IntoResponse`, so you can return it from
/// handlers the same way you'd return `Json`.
pub struct MatrixJson<T>(pub T);

impl<T> FromRequest<crate::state::AppState> for MatrixJson<T>
where
    T: DeserializeOwned,
{
    type Rejection = MatrixError;

    async fn from_request(
        req: Request,
        _state: &crate::state::AppState,
    ) -> Result<Self, Self::Rejection> {
        // Check Content-Type
        let content_type = req
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if !content_type.contains("application/json") {
            return Err(MatrixError::not_json());
        }

        // Read body
        let bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024) // 1MB limit
            .await
            .map_err(|_| MatrixError::bad_json("Failed to read request body"))?;

        // Validate UTF-8 before JSON parsing
        if std::str::from_utf8(&bytes).is_err() {
            return Err(MatrixError::not_json());
        }

        // Parse JSON
        let value: T = serde_json::from_slice(&bytes)
            .map_err(|e| MatrixError::bad_json(format!("Invalid JSON: {e}")))?;

        Ok(MatrixJson(value))
    }
}

/// Allow MatrixJson to be used as a response (like axum::Json).
impl<T: serde::Serialize> IntoResponse for MatrixJson<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}
