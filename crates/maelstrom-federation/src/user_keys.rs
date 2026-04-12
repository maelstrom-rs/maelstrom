//! # Cross-Server Device Key Queries
//!
//! End-to-end encryption (E2EE) in Matrix requires that clients know the device keys
//! of every user they share an encrypted room with. When those users are on a remote
//! server, the local server must query the remote server for their device keys.
//!
//! ## How It Works
//!
//! `POST /_matrix/federation/v1/user/keys/query` accepts a request with a
//! `device_keys` object mapping user IDs to lists of requested device IDs.
//! For example:
//!
//! ```json
//! {
//!   "device_keys": {
//!     "@alice:example.com": [],
//!     "@bob:example.com": ["DEVICEID1"]
//!   }
//! }
//! ```
//!
//! An empty array means "return keys for all devices." The server only returns
//! keys for users that actually belong to it (matching the server name in the
//! user ID).
//!
//! ## Response
//!
//! The response includes three sections:
//!
//! - `device_keys` -- per-device Curve25519 and Ed25519 keys, plus any signed keys
//! - `master_keys` -- cross-signing master keys (used to verify the user's identity)
//! - `self_signing_keys` -- cross-signing self-signing keys (used to sign device keys)
//!
//! These keys are essential for clients to establish Olm/Megolm sessions and verify
//! device trust.

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::server_name_from_sigil_id;

use crate::FederationState;

/// Build the user keys sub-router with the device key query endpoint.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route(
            "/_matrix/federation/v1/user/keys/query",
            post(query_user_keys),
        )
        .route(
            "/_matrix/federation/v1/user/devices/{userId}",
            get(get_user_devices),
        )
}

/// Request body for the federation device key query.
///
/// The `device_keys` field maps user IDs to lists of requested device IDs.
/// An empty list means "return all devices for this user."
#[derive(Deserialize)]
struct KeysQueryRequest {
    /// Map of user ID to list of device IDs. Empty list = all devices.
    device_keys: serde_json::Value,
}

/// POST /_matrix/federation/v1/user/keys/query
/// Query device keys for users on this server.
async fn query_user_keys(
    State(state): State<FederationState>,
    Json(body): Json<KeysQueryRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let device_keys_request = body
        .device_keys
        .as_object()
        .ok_or_else(|| MatrixError::bad_json("device_keys must be an object"))?;

    debug!(
        users = device_keys_request.len(),
        "Federation user keys query"
    );

    // Only serve keys for users on our server
    let our_server = state.server_name().as_str();
    let mut result_keys = serde_json::Map::new();

    for (user_id, _requested_devices) in device_keys_request {
        // Check if user belongs to our server
        let user_server = user_id.split(':').nth(1).unwrap_or("");
        if user_server != our_server {
            continue;
        }

        // Get device keys for this user
        match state
            .storage()
            .get_device_keys(std::slice::from_ref(user_id))
            .await
        {
            Ok(keys) => {
                if let Some(user_keys) = keys.get(user_id) {
                    result_keys.insert(user_id.clone(), user_keys.clone());
                }
            }
            Err(e) => {
                debug!(user_id = %user_id, error = %e, "Failed to get device keys");
            }
        }
    }

    // Also include cross-signing keys (master, self_signing)
    let mut master_keys = serde_json::Map::new();
    let mut self_signing_keys = serde_json::Map::new();

    for user_id in device_keys_request.keys() {
        let user_server = user_id.split(':').nth(1).unwrap_or("");
        if user_server != our_server {
            continue;
        }

        if let Ok(cross_keys) = state.storage().get_cross_signing_keys(user_id).await {
            if let Some(master) = cross_keys.get("master_key") {
                master_keys.insert(user_id.clone(), master.clone());
            }
            if let Some(self_signing) = cross_keys.get("self_signing_key") {
                self_signing_keys.insert(user_id.clone(), self_signing.clone());
            }
        }
    }

    Ok(Json(serde_json::json!({
        "device_keys": result_keys,
        "master_keys": master_keys,
        "self_signing_keys": self_signing_keys,
    })))
}

/// GET /_matrix/federation/v1/user/devices/{userId}
///
/// Returns all device information for a local user, including device keys.
/// Per Server-Server API section 2.7.
async fn get_user_devices(
    State(state): State<FederationState>,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    // Check user is on our server
    let server = server_name_from_sigil_id(&user_id);
    if server != state.server_name().as_str() {
        return Err(MatrixError::not_found("User not on this server"));
    }

    let user_id_typed = maelstrom_core::matrix::id::UserId::parse(&user_id)
        .map_err(|_| MatrixError::not_found("Invalid user ID"))?;

    let devices = state
        .storage()
        .list_devices(&user_id_typed)
        .await
        .unwrap_or_default();

    // Get device keys for this user (returns user_id -> { device_id -> keys })
    let all_keys = state
        .storage()
        .get_device_keys(std::slice::from_ref(&user_id))
        .await
        .unwrap_or(json!({}));
    let user_keys = all_keys.get(&user_id).cloned().unwrap_or(json!({}));

    let mut device_list = Vec::new();
    for device in &devices {
        let keys = user_keys
            .get(&device.device_id)
            .cloned()
            .unwrap_or(json!({}));
        device_list.push(json!({
            "device_id": device.device_id,
            "keys": keys,
            "device_display_name": device.display_name,
        }));
    }

    let stream_id = state.storage().current_stream_position().await.unwrap_or(1);

    debug!(
        user_id = %user_id,
        devices = device_list.len(),
        "Federation user devices query"
    );

    Ok(Json(json!({
        "user_id": user_id,
        "stream_id": stream_id,
        "devices": device_list,
    })))
}
