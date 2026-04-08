use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};

use maelstrom_core::error::MatrixError;
use maelstrom_core::identifiers::UserId;

use crate::extractors::{storage_error, AuthenticatedUser};
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
    Path((event_type, _txn_id)): Path<(String, String)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    if let Some(messages) = body.get("messages").and_then(|v| v.as_object()) {
        for (target_user, devices) in messages {
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
        }
    }

    Ok(Json(serde_json::json!({})))
}
