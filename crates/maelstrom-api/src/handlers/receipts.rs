//! Read receipts.
//!
//! Receipts let users indicate how far they have read in a room. When a client
//! sends a receipt for an event, the server records it and distributes it to
//! other members via the `ephemeral` section of `/sync` as `m.receipt` events.
//!
//! The primary receipt type is `m.read`, which updates the read marker visible
//! to other users. There is also `m.read.private` (visible only to the sender)
//! and `m.fully_read` (the "read marker" line in the timeline, set via
//! `/read_markers`).
//!
//! Receipts are **ephemeral events** -- they are not part of the room's
//! persistent event DAG, though the server stores the latest receipt per-user
//! to include in future `/sync` responses.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST` | `/_matrix/client/v3/rooms/{roomId}/receipt/{receiptType}/{eventId}` | Send a receipt marking the given event as read |
//!
//! # Matrix spec
//!
//! * [Receipts](https://spec.matrix.org/v1.18/client-server-api/#receipts)

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::timestamp_ms;
use maelstrom_core::matrix::id::server_name_from_sigil_id;
use maelstrom_core::matrix::room::Membership;

use crate::extractors::AuthenticatedUser;
use crate::handlers::util::require_membership;
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
    body: Option<axum::Json<serde_json::Value>>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is in the room
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Extract thread_id from request body (MSC4102).
    // Empty string means unthreaded (the default).
    let thread_id = body
        .as_ref()
        .and_then(|b| b.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    storage
        .set_receipt(&sender, &room_id, &receipt_type, &event_id, thread_id)
        .await
        .map_err(|e| MatrixError::unknown(e.to_string()))?;

    state
        .notifier()
        .notify(Notification::Receipt {
            room_id: room_id.clone(),
        })
        .await;

    // Queue m.receipt EDU to remote servers that share this room
    if let Some(tx_sender) = state.transaction_sender() {
        let local_server = state.server_name().as_str();
        let mut remote_servers = std::collections::HashSet::new();
        if let Ok(members) = storage
            .get_room_members(&room_id, Membership::Join.as_str())
            .await
        {
            for member in members {
                let server = server_name_from_sigil_id(&member);
                if !server.is_empty() && server != local_server {
                    remote_servers.insert(server.to_string());
                }
            }
        }

        let ts = timestamp_ms();
        for server in remote_servers {
            tx_sender.queue_edu(
                &server,
                serde_json::json!({
                    "edu_type": "m.receipt",
                    "content": {
                        &room_id: {
                            &receipt_type: {
                                &sender: {
                                    "event_ids": [&event_id],
                                    "data": { "ts": ts }
                                }
                            }
                        }
                    }
                }),
            );
        }
    }

    Ok(Json(serde_json::json!({})))
}
