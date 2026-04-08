use axum::extract::{Path, State};
use axum::routing::put;
use axum::{Json, Router};
use serde::Deserialize;
use tracing::{debug, warn};

use maelstrom_core::error::MatrixError;
use maelstrom_core::events::pdu::StoredEvent;

use crate::FederationState;

pub fn routes() -> Router<FederationState> {
    Router::new().route(
        "/_matrix/federation/v1/send/{txnId}",
        put(receive_transaction),
    )
}

#[derive(Deserialize)]
struct Transaction {
    origin: String,
    #[allow(dead_code)]
    origin_server_ts: Option<u64>,
    #[serde(default)]
    pdus: Vec<serde_json::Value>,
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

/// Process a single inbound PDU.
async fn process_pdu(
    state: &FederationState,
    pdu_json: &serde_json::Value,
    _origin: &str,
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

    // Check if event already exists
    if state.storage().get_event(event_id).await.is_ok() {
        debug!(event_id = %event_id, "Event already exists, skipping");
        return Ok(());
    }

    // Build StoredEvent from PDU
    let stored = StoredEvent {
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
                                    .set_receipt(user_id, room_id, "m.read", event_id)
                                    .await;
                            }
                        }
                    }
                }
            }
        }
        "m.device_list_update" => {
            // Device list updates inform us a remote user's device keys changed.
            // We invalidate our cache by storing the updated keys if provided.
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
        }
        other => {
            debug!(edu_type = %other, "Unhandled federation EDU type");
        }
    }
}
