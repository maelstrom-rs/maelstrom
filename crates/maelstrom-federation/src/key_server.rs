//! # Server Signing Key Distribution
//!
//! Every Matrix homeserver has an **Ed25519 signing key** that it uses to sign events
//! and federation requests. Other servers need to fetch this public key to verify
//! those signatures. This module implements the key distribution endpoints.
//!
//! ## How It Works
//!
//! 1. Each server publishes its keys at `GET /_matrix/key/v2/server`. The response
//!    is a self-signed JSON object containing:
//!    - `server_name` -- the server's canonical name
//!    - `verify_keys` -- a map of key ID to public key (e.g., `{"ed25519:abc": {"key": "<base64>"}}`)
//!    - `old_verify_keys` -- previously valid keys that may still be needed to verify old events
//!    - `valid_until_ts` -- Unix timestamp (ms) after which the keys should be re-fetched
//!
//! 2. The response is **self-signed** -- the server signs the entire key response JSON
//!    with its own key, so recipients can verify authenticity.
//!
//! 3. Servers can also act as **notaries** -- proxying key lookups for other servers.
//!    `GET /_matrix/key/v2/query/{serverName}` fetches and returns another server's keys,
//!    and `POST /_matrix/key/v2/query` supports batch queries for multiple servers at once.
//!
//! ## Endpoints
//!
//! - `GET /_matrix/key/v2/server` -- return this server's own signing keys
//! - `GET /_matrix/key/v2/server/{keyId}` -- same (key ID is informational only)
//! - `GET /_matrix/key/v2/query/{serverName}` -- notary: fetch another server's keys
//! - `POST /_matrix/key/v2/query` -- notary: batch query multiple servers
//! - `GET /_matrix/federation/v1/version` -- server name and version info

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::signing::sign_event;
use tracing::debug;

use crate::FederationState;

/// Build the key server sub-router with all key distribution endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route("/_matrix/key/v2/server", get(get_server_keys))
        .route("/_matrix/key/v2/server/{keyId}", get(get_server_keys))
        .route("/_matrix/key/v2/query/{serverName}", get(query_server_keys))
        .route("/_matrix/key/v2/query", post(query_server_keys_batch))
        .route("/_matrix/federation/v1/version", get(get_version))
}

/// GET /_matrix/federation/v1/version — server version info.
async fn get_version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "server": {
            "name": "Maelstrom",
            "version": env!("CARGO_PKG_VERSION"),
        }
    }))
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
        let valid_until_dt =
            chrono::DateTime::from_timestamp_millis(valid_until).unwrap_or_else(chrono::Utc::now);

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
