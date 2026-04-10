//! # Inbound Federation Transaction Processing
//!
//! This module handles the receiving side of federation: when a remote server sends
//! us a transaction via `PUT /_matrix/federation/v1/send/{txnId}`, this code
//! processes it.
//!
//! ## Transaction Structure
//!
//! An inbound transaction contains:
//!
//! - `origin` -- the server that sent the transaction
//! - `origin_server_ts` -- when the transaction was created
//! - `pdus` -- an array of Persistent Data Units (room events) to be stored
//! - `edus` -- an array of Ephemeral Data Units (typing, presence, receipts, etc.)
//!
//! ## Processing Pipeline
//!
//! 1. **Transaction deduplication** -- if we have already processed a transaction with
//!    the same `(origin, txnId)` pair, return a cached empty result immediately. This
//!    prevents duplicate processing when a remote server retries.
//!
//! 2. **PDU processing** -- each PDU is validated, converted to a [`Pdu`] struct, and
//!    stored. If the PDU is a state event (has a `state_key`), the room's current
//!    state is updated. Already-known events (by event ID) are silently skipped.
//!
//! 3. **EDU processing** -- each EDU is dispatched by `edu_type`:
//!    - `m.typing` -- updates the ephemeral typing state
//!    - `m.presence` -- updates user presence status
//!    - `m.receipt` -- stores read receipts
//!    - `m.device_list_update` -- stores updated device keys for remote users
//!
//! 4. **Transaction recording** -- the `(origin, txnId)` pair is stored to support
//!    deduplication on retries.

use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use base64::Engine;
use serde::Deserialize;
use tracing::{debug, warn};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::Pdu;
use maelstrom_storage::traits::RemoteKeyRecord;

use crate::FederationState;

/// Build the receiver sub-router with the inbound transaction endpoint.
pub fn routes() -> Router<FederationState> {
    Router::new().route(
        "/_matrix/federation/v1/send/{txnId}",
        put(receive_transaction),
    )
}

/// A federation transaction received from a remote server.
///
/// Contains batches of PDUs (persistent room events) and EDUs (ephemeral data like
/// typing notifications). The `origin` identifies the sending server, and PDUs/EDUs
/// default to empty arrays if omitted.
#[derive(Deserialize)]
struct Transaction {
    /// The server name that originated this transaction.
    origin: String,
    /// Timestamp when the transaction was created (informational, not used for ordering).
    #[allow(dead_code)]
    origin_server_ts: Option<u64>,
    /// Persistent Data Units -- room events to be stored and processed.
    #[serde(default)]
    pdus: Vec<serde_json::Value>,
    /// Ephemeral Data Units -- transient data (typing, presence, receipts, device updates).
    #[serde(default)]
    edus: Vec<serde_json::Value>,
}

/// PUT /_matrix/federation/v1/send/{txnId} — receive inbound transactions.
async fn receive_transaction(
    State(state): State<FederationState>,
    Path(txn_id): Path<String>,
    Json(txn): Json<Transaction>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(
        origin = %txn.origin,
        txn_id = %txn_id,
        pdus = txn.pdus.len(),
        edus = txn.edus.len(),
        "Received federation transaction"
    );

    // Transaction deduplication
    if state
        .storage()
        .has_federation_txn(&txn.origin, &txn_id)
        .await
        .unwrap_or(false)
    {
        debug!(txn_id = %txn_id, "Duplicate transaction, returning cached result");
        return Ok(Json(serde_json::json!({ "pdus": {} })));
    }

    let mut pdu_results = serde_json::Map::new();

    // Process PDUs
    for pdu_json in &txn.pdus {
        let event_id = pdu_json
            .get("event_id")
            .and_then(|e| e.as_str())
            .unwrap_or_default()
            .to_string();

        match process_pdu(&state, pdu_json, &txn.origin).await {
            Ok(()) => {
                pdu_results.insert(event_id, serde_json::json!({}));
            }
            Err(e) => {
                warn!(event_id = %event_id, error = %e, "Failed to process PDU");
                pdu_results.insert(event_id, serde_json::json!({ "error": e.to_string() }));
            }
        }
    }

    // Process EDUs
    for edu in &txn.edus {
        process_edu(&state, edu).await;
    }

    // Record transaction
    let _ = state
        .storage()
        .store_federation_txn(&txn.origin, &txn_id)
        .await;

    Ok(Json(serde_json::json!({ "pdus": pdu_results })))
}

/// Fetch a remote server's Ed25519 public key, using cache when available.
///
/// 1. Check local cache (`FederationKeyStore::get_remote_server_keys`)
/// 2. If not cached or expired, fetch from the remote server via `/_matrix/key/v2/server`
/// 3. Cache the fetched keys for future use
/// 4. Return the public key bytes for the requested `key_id`
async fn resolve_server_key(
    state: &FederationState,
    server_name: &str,
    key_id: &str,
) -> Option<[u8; 32]> {
    let b64_engine = base64::engine::general_purpose::STANDARD_NO_PAD;

    // 1. Check local cache
    if let Ok(cached_keys) = state.storage().get_remote_server_keys(server_name).await {
        for record in &cached_keys {
            if record.key_id == key_id
                && record.valid_until > chrono::Utc::now()
                && let Ok(bytes) = b64_engine.decode(&record.public_key)
                && let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice())
            {
                return Some(arr);
            }
        }
    }

    // 2. Fetch from remote server
    let keys_response = match state.client().fetch_server_keys(server_name).await {
        Ok(resp) => resp,
        Err(e) => {
            warn!(
                server_name = %server_name,
                error = %e,
                "Failed to fetch server keys for signature verification"
            );
            return None;
        }
    };

    // 3. Parse and cache the keys
    let valid_until_ts = keys_response
        .get("valid_until_ts")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let valid_until = chrono::DateTime::from_timestamp_millis(valid_until_ts as i64)
        .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(24));

    let mut records = Vec::new();
    let mut result_key: Option<[u8; 32]> = None;

    if let Some(verify_keys) = keys_response.get("verify_keys").and_then(|v| v.as_object()) {
        for (kid, key_data) in verify_keys {
            if let Some(pub_key_b64) = key_data.get("key").and_then(|k| k.as_str()) {
                records.push(RemoteKeyRecord {
                    server_name: server_name.to_string(),
                    key_id: kid.clone(),
                    public_key: pub_key_b64.to_string(),
                    valid_until,
                });

                if kid == key_id
                    && let Ok(bytes) = b64_engine.decode(pub_key_b64)
                    && let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice())
                {
                    result_key = Some(arr);
                }
            }
        }
    }

    // Also check old_verify_keys in case the key rotated but we still need it
    if result_key.is_none()
        && let Some(old_keys) = keys_response
            .get("old_verify_keys")
            .and_then(|v| v.as_object())
    {
        for (kid, key_data) in old_keys {
            let old_valid_until_ts = key_data
                .get("expired_ts")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let old_valid_until =
                chrono::DateTime::from_timestamp_millis(old_valid_until_ts as i64)
                    .unwrap_or_else(chrono::Utc::now);

            if let Some(pub_key_b64) = key_data.get("key").and_then(|k| k.as_str()) {
                records.push(RemoteKeyRecord {
                    server_name: server_name.to_string(),
                    key_id: kid.clone(),
                    public_key: pub_key_b64.to_string(),
                    valid_until: old_valid_until,
                });

                if kid == key_id
                    && let Ok(bytes) = b64_engine.decode(pub_key_b64)
                    && let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice())
                {
                    result_key = Some(arr);
                }
            }
        }
    }

    // Store all fetched keys in cache
    if !records.is_empty()
        && let Err(e) = state.storage().store_remote_server_keys(&records).await
    {
        warn!(
            server_name = %server_name,
            error = %e,
            "Failed to cache remote server keys"
        );
    }

    result_key
}

/// Process a single inbound PDU.
async fn process_pdu(
    state: &FederationState,
    pdu_json: &serde_json::Value,
    origin: &str,
) -> Result<(), MatrixError> {
    // Extract required fields
    let event_id = pdu_json
        .get("event_id")
        .and_then(|e| e.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing event_id"))?;

    let room_id = pdu_json
        .get("room_id")
        .and_then(|e| e.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing room_id"))?;

    let sender = pdu_json
        .get("sender")
        .and_then(|e| e.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing sender"))?;

    let event_type = pdu_json
        .get("type")
        .and_then(|e| e.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing type"))?;

    let content = pdu_json
        .get("content")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let origin_server_ts = pdu_json
        .get("origin_server_ts")
        .and_then(|e| e.as_u64())
        .unwrap_or(0);

    // Check server ACL -- deny the origin server if the room's ACL blocks it
    check_server_acl(state.storage(), room_id, origin).await?;

    // Check if event already exists
    if state.storage().get_event(event_id).await.is_ok() {
        debug!(event_id = %event_id, "Event already exists, skipping");
        return Ok(());
    }

    // Verify event signature from the origin server
    if let Some(sigs) = pdu_json.get("signatures").and_then(|s| s.as_object()) {
        let signing_server = origin;
        if let Some(server_sigs) = sigs.get(signing_server).and_then(|s| s.as_object()) {
            let mut verified = false;
            for (key_id, _sig) in server_sigs {
                if let Some(public_key) = resolve_server_key(state, signing_server, key_id).await
                    && maelstrom_core::matrix::signing::verify_event_signature(
                        pdu_json,
                        &public_key,
                        signing_server,
                        key_id,
                    )
                {
                    verified = true;
                    break;
                }
            }
            if !verified {
                warn!(
                    event_id = %event_id,
                    origin = %origin,
                    "Event signature verification failed"
                );
                return Err(MatrixError::forbidden(
                    "Event signature verification failed",
                ));
            }
        } else {
            // No signature from the origin server — warn but allow for now.
            // Third-party invites and some edge cases may have signatures from
            // a different server than the transaction origin.
            warn!(
                event_id = %event_id,
                origin = %origin,
                "No signature from origin server on event"
            );
        }
    } else {
        warn!(
            event_id = %event_id,
            "Event has no signatures field"
        );
    }

    // Build Pdu from incoming event
    let stored = Pdu {
        event_id: event_id.to_string(),
        room_id: room_id.to_string(),
        sender: sender.to_string(),
        event_type: event_type.to_string(),
        state_key: pdu_json
            .get("state_key")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        content,
        origin_server_ts,
        unsigned: pdu_json.get("unsigned").cloned(),
        stream_position: 0, // Will be set by store_event
        origin: pdu_json
            .get("origin")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        auth_events: pdu_json.get("auth_events").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        prev_events: pdu_json.get("prev_events").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        depth: pdu_json.get("depth").and_then(|d| d.as_i64()),
        hashes: pdu_json.get("hashes").cloned(),
        signatures: pdu_json.get("signatures").cloned(),
    };

    // Store the event
    state.storage().store_event(&stored).await.map_err(|e| {
        tracing::error!(event_id = %event_id, error = %e, "Failed to store federated event");
        MatrixError::unknown("Failed to store event")
    })?;

    // If it's a state event, update room state
    if let Some(state_key) = &stored.state_key {
        let _ = state
            .storage()
            .set_room_state(room_id, event_type, state_key, event_id)
            .await;
    }

    debug!(event_id = %event_id, room_id = %room_id, "Stored federated event");
    Ok(())
}

/// Process an inbound EDU (Ephemeral Data Unit).
///
/// EDUs carry transient information that is not persisted as room events.
/// Supported types:
/// - `m.typing` -- a user started or stopped typing in a room
/// - `m.presence` -- a batch of presence updates for remote users
/// - `m.receipt` -- read receipts for events in shared rooms
/// - `m.device_list_update` -- a remote user's device keys changed (important for E2EE)
async fn process_edu(state: &FederationState, edu: &serde_json::Value) {
    let edu_type = edu
        .get("edu_type")
        .and_then(|e| e.as_str())
        .unwrap_or("unknown");
    let content = edu.get("content").cloned().unwrap_or(serde_json::json!({}));

    match edu_type {
        "m.typing" => {
            let room_id = content
                .get("room_id")
                .and_then(|r| r.as_str())
                .unwrap_or_default();
            let user_id = content
                .get("user_id")
                .and_then(|u| u.as_str())
                .unwrap_or_default();
            let typing = content
                .get("typing")
                .and_then(|t| t.as_bool())
                .unwrap_or(false);
            debug!(room_id = %room_id, user_id = %user_id, typing = typing, "Federation typing EDU");
            state
                .ephemeral()
                .set_typing(user_id, room_id, typing, 30_000);
        }
        "m.presence" => {
            if let Some(push) = content.get("push").and_then(|p| p.as_array()) {
                for entry in push {
                    let user_id = entry
                        .get("user_id")
                        .and_then(|u| u.as_str())
                        .unwrap_or_default();
                    let presence = entry
                        .get("presence")
                        .and_then(|p| p.as_str())
                        .unwrap_or("offline");
                    let status_msg = entry.get("status_msg").and_then(|s| s.as_str());
                    debug!(user_id = %user_id, presence = %presence, "Federation presence EDU");
                    state
                        .ephemeral()
                        .set_presence(user_id, presence, status_msg);
                }
            }
        }
        "m.receipt" => {
            let room_id = content
                .get("room_id")
                .and_then(|r| r.as_str())
                .unwrap_or_default();
            if let Some(receipts) = content.get("m.read").and_then(|r| r.as_object()) {
                for (event_id, data) in receipts {
                    if let Some(user_ids) = data.get("user_ids").and_then(|u| u.as_array()) {
                        for uid in user_ids {
                            if let Some(user_id) = uid.as_str() {
                                debug!(room_id = %room_id, user_id = %user_id, event_id = %event_id, "Federation receipt EDU");
                                let _ = state
                                    .storage()
                                    .set_receipt(user_id, room_id, "m.read", event_id, "")
                                    .await;
                            }
                        }
                    }
                }
            }
        }
        "m.device_list_update" => {
            // Device list updates inform us a remote user's device keys changed.
            let user_id = content
                .get("user_id")
                .and_then(|u| u.as_str())
                .unwrap_or_default();
            let device_id = content
                .get("device_id")
                .and_then(|d| d.as_str())
                .unwrap_or_default();
            debug!(user_id = %user_id, device_id = %device_id, "Federation device list update EDU");
            if let Some(keys) = content.get("keys") {
                let _ = state
                    .storage()
                    .set_device_keys(user_id, device_id, keys)
                    .await;
            }
            // Record the change position so sync's device_lists.changed picks it up
            if !user_id.is_empty() {
                let change_pos = state.storage().current_stream_position().await.unwrap_or(0);
                let _ = state
                    .storage()
                    .set_account_data(
                        user_id,
                        None,
                        "_maelstrom.device_change_pos",
                        &serde_json::json!({"pos": change_pos}),
                    )
                    .await;
            }
        }
        other => {
            debug!(edu_type = %other, "Unhandled federation EDU type");
        }
    }
}

/// Check if a server is allowed by the room's `m.room.server_acl` state event.
async fn check_server_acl(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    server_name: &str,
) -> Result<(), MatrixError> {
    use maelstrom_core::matrix::room::event_type as et;
    let acl = match storage.get_state_event(room_id, et::SERVER_ACL, "").await {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    maelstrom_core::matrix::room::server_acl_allowed(&acl.content, server_name)
        .then_some(())
        .ok_or_else(|| MatrixError::forbidden("Server denied by room ACL"))
}
