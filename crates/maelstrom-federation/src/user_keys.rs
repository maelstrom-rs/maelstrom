use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::error::MatrixError;

use crate::FederationState;

pub fn routes() -> Router<FederationState> {
    Router::new().route(
        "/_matrix/federation/v1/user/keys/query",
        post(query_user_keys),
    )
}

#[derive(Deserialize)]
struct KeysQueryRequest {
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
