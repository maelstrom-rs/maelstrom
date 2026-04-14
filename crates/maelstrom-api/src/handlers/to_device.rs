//! Send-to-device messaging.
//!
//! To-device messages are delivered directly to specific devices rather than
//! being broadcast to a room. They are the primary transport for E2EE key
//! exchange: when a client needs to establish an Olm session with another
//! device, it sends the initial key-exchange payload as a to-device event
//! (e.g. `m.room_key`, `m.room.encrypted`).
//!
//! Other uses include verification requests (`m.key.verification.*`) and
//! secret sharing between a user's own devices.
//!
//! To-device messages are **not** part of any room's event DAG. They are queued
//! per-device and delivered via the `to_device` section of `/sync`, then
//! deleted once acknowledged.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `PUT` | `/_matrix/client/v3/sendToDevice/{eventType}/{txnId}` | Send to-device events to a map of users and devices |
//!
//! # Matrix spec
//!
//! * [Send-to-Device messaging](https://spec.matrix.org/v1.18/client-server-api/#send-to-device-messaging)

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::{UserId, server_name_from_sigil_id};

use crate::extractors::{AuthenticatedUser, storage_error};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v3/sendToDevice/{eventType}/{txnId}",
        put(send_to_device),
    )
}

/// PUT /_matrix/client/v3/sendToDevice/{eventType}/{txnId}
///
/// Send to-device events to specific devices.
async fn send_to_device(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((event_type, txn_id)): Path<(String, String)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();
    let local_server = state.server_name().as_str();

    // Collect remote messages grouped by destination server for federation relay.
    // Key: server_name -> { target_user -> { device_id -> content } }
    let mut remote_messages: HashMap<
        String,
        HashMap<String, serde_json::Map<String, serde_json::Value>>,
    > = HashMap::new();

    if let Some(messages) = body.get("messages").and_then(|v| v.as_object()) {
        for (target_user, devices) in messages {
            let target_server = server_name_from_sigil_id(target_user);

            if target_server == local_server {
                // Local user -- store messages directly
                if let Some(device_map) = devices.as_object() {
                    for (target_device, content) in device_map {
                        if target_device == "*" {
                            // Broadcast to all devices for this user
                            let user_devices = storage
                                .list_devices(&UserId::parse(target_user).map_err(|_| {
                                    MatrixError::unknown(format!("Invalid user ID: {target_user}"))
                                })?)
                                .await
                                .map_err(storage_error)?;

                            for device in user_devices {
                                storage
                                    .store_to_device(
                                        target_user,
                                        &device.device_id,
                                        &sender,
                                        &event_type,
                                        content,
                                    )
                                    .await
                                    .map_err(storage_error)?;
                            }
                        } else {
                            storage
                                .store_to_device(
                                    target_user,
                                    target_device,
                                    &sender,
                                    &event_type,
                                    content,
                                )
                                .await
                                .map_err(storage_error)?;
                        }
                    }
                }
            } else if !target_server.is_empty() {
                // Remote user -- collect for federation relay
                if let Some(device_map) = devices.as_object() {
                    let user_entry = remote_messages
                        .entry(target_server.to_string())
                        .or_default()
                        .entry(target_user.clone())
                        .or_default();
                    for (device_id, content) in device_map {
                        user_entry.insert(device_id.clone(), content.clone());
                    }
                }
            }
        }
    }

    // Queue m.direct_to_device EDUs to remote servers
    if let Some(tx_sender) = state.transaction_sender() {
        for (server, user_device_messages) in remote_messages {
            let messages_value: serde_json::Value =
                serde_json::to_value(&user_device_messages).unwrap_or_default();
            tx_sender.queue_edu(
                &server,
                serde_json::json!({
                    "edu_type": "m.direct_to_device",
                    "content": {
                        "sender": &sender,
                        "type": &event_type,
                        "message_id": &txn_id,
                        "messages": messages_value,
                    }
                }),
            );
        }
    }

    Ok(Json(serde_json::json!({})))
}
