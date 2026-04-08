use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v3/rooms/{roomId}/typing/{userId}",
        put(set_typing),
    )
}

#[derive(Deserialize)]
struct TypingRequest {
    typing: bool,
    #[serde(default)]
    timeout: Option<u64>,
}

async fn set_typing(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, user_id)): Path<(String, String)>,
    MatrixJson(body): MatrixJson<TypingRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    // Check that the authenticated user matches the path user
    if sender != user_id {
        return Err(MatrixError::forbidden(
            "Cannot set typing for another user",
        ));
    }

    let timeout_ms = body.timeout.unwrap_or(30000);

    state
        .ephemeral()
        .set_typing(&sender, &room_id, body.typing, timeout_ms);

    state
        .notifier()
        .notify(Notification::Typing {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}
