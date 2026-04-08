use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use maelstrom_core::error::MatrixError;

use crate::auth::AdminUser;
use crate::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/_maelstrom/admin/v1/federation/stats", get(federation_stats))
}

async fn federation_stats(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let active_keys = state
        .storage()
        .get_active_server_keys()
        .await
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "server_name": state.server_name().as_str(),
        "signing_keys": active_keys.len(),
        "status": "operational",
    })))
}
