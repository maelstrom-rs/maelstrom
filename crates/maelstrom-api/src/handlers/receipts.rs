use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};

use maelstrom_core::error::MatrixError;

use crate::extractors::AuthenticatedUser;
use crate::notify::Notification;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v3/rooms/{roomId}/receipt/{receiptType}/{eventId}",
        post(send_receipt),
    )
}

async fn send_receipt(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, receipt_type, event_id)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is in the room
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|_| MatrixError::forbidden("You are not in this room"))?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    storage
        .set_receipt(&sender, &room_id, &receipt_type, &event_id)
        .await
        .map_err(|e| MatrixError::unknown(e.to_string()))?;

    state
        .notifier()
        .notify(Notification::Receipt {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}
