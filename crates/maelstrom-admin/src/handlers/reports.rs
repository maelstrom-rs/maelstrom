//! Content report review and moderation queue.
//!
//! Provides endpoints for reviewing user-submitted content reports (abuse
//! reports filed via the Client-Server API's `POST /_matrix/client/v3/rooms/{roomId}/report/{eventId}`).
//! All endpoints require admin authentication.
//!
//! ## Routes
//!
//! | Method | Path                                 | Operation           |
//! |--------|--------------------------------------|---------------------|
//! | `GET`  | `/_maelstrom/admin/v1/reports`       | List content reports |
//!
//! This is currently a placeholder that returns an empty list. The full
//! implementation will query the `event_report` table with pagination and
//! filtering support.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use maelstrom_core::matrix::error::MatrixError;

use crate::AdminState;
use crate::auth::AdminUser;

pub fn routes() -> Router<AdminState> {
    Router::new().route("/_maelstrom/admin/v1/reports", get(list_reports))
}

async fn list_reports(
    State(_state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    // Reports are stored in event_report table
    // Full implementation would query with pagination
    Ok(Json(serde_json::json!({
        "reports": [],
        "total": 0,
    })))
}
