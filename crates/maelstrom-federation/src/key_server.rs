use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use maelstrom_core::error::MatrixError;
use maelstrom_core::signatures::sign_event;
use tracing::debug;

use crate::FederationState;

pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/key/v2/server", get(get_server_keys))
        .route("/_matrix/key/v2/server/{keyId}", get(get_server_keys))
        .route(
            "/_matrix/key/v2/query/{serverName}",
            get(query_server_keys),
        )
        .route("/_matrix/key/v2/query", post(query_server_keys_batch))
}

/// GET /_matrix/key/v2/server — return this server's signing keys (self-signed).
async fn get_server_keys(
    State(state): State<FederationState>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let key = state.signing_key();
    let server_name = state.server_name().as_str();

    // Valid for 7 days from now
    let valid_until = chrono::Utc::now() + chrono::Duration::days(7);
    let valid_until_ts = valid_until.timestamp_millis();

    let response = serde_json::json!({
        "server_name": server_name,
        "verify_keys": {
            key.key_id(): {
                "key": key.public_key_base64(),
            }
        },
        "old_verify_keys": {},
        "valid_until_ts": valid_until_ts,
    });

    // Self-sign the key response
    let signed = sign_event(&response, key, server_name);

    Ok(Json(signed))
}

/// GET /_matrix/key/v2/query/{serverName} — notary: fetch and return another server's keys.
async fn query_server_keys(
    State(state): State<FederationState>,
    Path(target_server): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(server = %target_server, "Notary key query");

    // If asking about ourselves, return our own keys
    if target_server == state.server_name().as_str() {
        return get_server_keys(State(state)).await;
    }

    // Fetch from the target server
    let keys = state
        .client()
        .fetch_server_keys(&target_server)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to fetch keys for notary query");
            MatrixError::not_found("Could not retrieve keys for server")
        })?;

    // Cache the remote keys
    if let Some(verify_keys) = keys.get("verify_keys").and_then(|v| v.as_object()) {
        let mut records = Vec::new();
        let valid_until = keys
            .get("valid_until_ts")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let valid_until_dt = chrono::DateTime::from_timestamp_millis(valid_until)
            .unwrap_or_else(chrono::Utc::now);

        for (key_id, key_data) in verify_keys {
            if let Some(pub_key) = key_data.get("key").and_then(|k| k.as_str()) {
                records.push(maelstrom_storage::traits::RemoteKeyRecord {
                    server_name: target_server.clone(),
                    key_id: key_id.clone(),
                    public_key: pub_key.to_string(),
                    valid_until: valid_until_dt,
                });
            }
        }

        let _ = state.storage().store_remote_server_keys(&records).await;
    }

    // Wrap in the notary response format
    Ok(Json(serde_json::json!({
        "server_keys": [keys],
    })))
}

/// POST /_matrix/key/v2/query — batch notary query.
async fn query_server_keys_batch(
    State(state): State<FederationState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let server_keys = body
        .get("server_keys")
        .and_then(|s| s.as_object())
        .ok_or_else(|| MatrixError::bad_json("Missing server_keys"))?;

    let mut results = Vec::new();

    for (server_name, _key_queries) in server_keys {
        if server_name == state.server_name().as_str() {
            // Return our own keys
            let key = state.signing_key();
            let valid_until = chrono::Utc::now() + chrono::Duration::days(7);
            let response = serde_json::json!({
                "server_name": server_name,
                "verify_keys": {
                    key.key_id(): {
                        "key": key.public_key_base64(),
                    }
                },
                "old_verify_keys": {},
                "valid_until_ts": valid_until.timestamp_millis(),
            });
            let signed = sign_event(&response, key, server_name);
            results.push(signed);
        } else {
            // Fetch from remote
            if let Ok(keys) = state.client().fetch_server_keys(server_name).await {
                results.push(keys);
            }
        }
    }

    Ok(Json(serde_json::json!({
        "server_keys": results,
    })))
}
