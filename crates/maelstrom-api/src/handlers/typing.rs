//! Typing indicators.
//!
//! Typing notifications are **ephemeral** -- they are never persisted to the
//! event graph and exist only in memory. When a user starts typing, their client
//! sends a request with `typing: true` and an optional `timeout` (default
//! ~30 seconds). The server holds this state and delivers it to other room
//! members via the `ephemeral` section of `/sync`.
//!
//! If the user stops typing (or the timeout expires without renewal), the
//! indicator is automatically cleared.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `PUT` | `/_matrix/client/v3/rooms/{roomId}/typing/{userId}` | Set or clear the typing indicator for a user in a room |
//!
//! # Matrix spec
//!
//! * [Typing notifications](https://spec.matrix.org/v1.12/client-server-api/#typing-notifications)

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;

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
        return Err(MatrixError::forbidden("Cannot set typing for another user"));
    }

    // Ensure a minimum timeout so typing doesn't expire before the next
    // sync can pick it up.  The spec default is ~30 s; clamp to at least 10 s.
    let timeout_ms = body.timeout.unwrap_or(30000).max(10000);

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
