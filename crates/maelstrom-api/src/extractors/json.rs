use axum::extract::{FromRequest, Request};
use axum::response::{IntoResponse, Response};
use maelstrom_core::error::MatrixError;
use serde::de::DeserializeOwned;

/// Matrix-spec-compliant JSON body extractor.
///
/// Returns `M_NOT_JSON` if Content-Type is not `application/json`.
/// Returns `M_BAD_JSON` if the body fails to parse.
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
