//! User presence.
//!
//! Presence tracks whether a user is currently active and available. Each user
//! has a presence state that is one of:
//!
//! * `online` -- the user is actively using a client right now.
//! * `unavailable` -- the user has been idle for a period of time.
//! * `offline` -- the user is not connected or has explicitly set offline.
//!
//! The server also tracks `last_active_ago` (milliseconds since the user was
//! last active) and an optional `status_msg` (a free-form human-readable
//! string like "In a meeting").
//!
//! Presence is ephemeral and held in memory; it is delivered to other users via
//! the `presence` section of `/sync`.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/_matrix/client/v3/presence/{userId}/status` | Get the presence state for a user |
//! | `PUT` | `/_matrix/client/v3/presence/{userId}/status` | Set the calling user's presence state |
//!
//! # Matrix spec
//!
//! * [Presence](https://spec.matrix.org/v1.12/client-server-api/#presence)

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v3/presence/{userId}/status",
        get(get_presence).put(set_presence),
    )
}

// -- GET /presence/{userId}/status --

async fn get_presence(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    match state.ephemeral().get_presence(&user_id) {
        Some(record) => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let last_active_ago = now_ms.saturating_sub(record.last_active_ts);

            let mut response = serde_json::json!({
                "presence": record.status,
                "last_active_ago": last_active_ago,
            });

            if let Some(msg) = &record.status_msg {
                response["status_msg"] = serde_json::Value::String(msg.clone());
            }

            Ok(Json(response))
        }
        None => {
            // Return default "offline" presence for users without explicit presence
            Ok(Json(serde_json::json!({
                "presence": "offline",
                "last_active_ago": 0,
            })))
        }
    }
}

// -- PUT /presence/{userId}/status --

#[derive(Deserialize)]
struct SetPresenceRequest {
    presence: String,
    status_msg: Option<String>,
}

async fn set_presence(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(user_id): Path<String>,
    MatrixJson(body): MatrixJson<SetPresenceRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    if sender != user_id {
        return Err(MatrixError::forbidden(
            "Cannot set presence for another user",
        ));
    }

    // Validate presence value
    match body.presence.as_str() {
        "online" | "offline" | "unavailable" => {}
        _ => {
            return Err(MatrixError::bad_json(
                "presence must be one of: online, offline, unavailable",
            ));
        }
    }

    state
        .ephemeral()
        .set_presence(&sender, &body.presence, body.status_msg.as_deref());

    state
        .notifier()
        .notify(Notification::Presence {
            user_id: sender.clone(),
        })
        .await;

    // Send presence EDU to remote servers sharing rooms with this user
    if let Some(tx_sender) = state.transaction_sender() {
        let remote_servers = crate::handlers::util::servers_sharing_rooms(
            state.storage(),
            &sender,
            state.server_name().as_str(),
        )
        .await;
        for server in remote_servers {
            let mut edu_content = serde_json::json!({
                "user_id": sender,
                "presence": body.presence,
                "last_active_ago": 0,
                "currently_active": body.presence == "online",
            });
            if let Some(ref msg) = body.status_msg {
                edu_content["status_msg"] = serde_json::Value::String(msg.clone());
            }
            tx_sender.queue_edu(
                &server,
                serde_json::json!({
                    "edu_type": "m.presence",
                    "content": edu_content,
                }),
            );
        }
    }

    Ok(Json(serde_json::json!({})))
}
