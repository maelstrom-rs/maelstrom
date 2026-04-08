use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use maelstrom_core::error::MatrixError;

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
