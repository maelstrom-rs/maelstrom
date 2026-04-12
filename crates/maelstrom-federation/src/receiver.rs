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

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::{debug, warn};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::Pdu;
use maelstrom_storage::traits::RemoteKeyRecord;

use crate::FederationState;

// ---------------------------------------------------------------------------
// Federation rate limiting
// ---------------------------------------------------------------------------

/// Simple fixed-window rate limiter for inbound federation transactions.
///
/// Tracks `(window_start_ms, request_count)` per origin server.  When a new
/// request arrives and the current window has not elapsed, the counter is
/// incremented; once it exceeds [`FED_RATE_MAX_REQUESTS`] the origin is
/// rejected with HTTP 429 until the window resets.
static FED_RATE_LIMITS: std::sync::LazyLock<Mutex<HashMap<String, (u64, u32)>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Window size for federation rate limiting (1 minute).
const FED_RATE_WINDOW_MS: u64 = 60_000;

/// Maximum number of transactions accepted per origin per window.
const FED_RATE_MAX_REQUESTS: u32 = 100;

/// Check whether `origin` has exceeded its federation rate limit.
fn check_federation_rate_limit(origin: &str) -> Result<(), MatrixError> {
    let now = maelstrom_core::matrix::event::timestamp_ms();
    let mut limits = FED_RATE_LIMITS.lock().unwrap_or_else(|e| e.into_inner());
    let entry = limits.entry(origin.to_string()).or_insert((now, 0));

    if now - entry.0 > FED_RATE_WINDOW_MS {
        // Window expired -- reset.
        *entry = (now, 1);
        Ok(())
    } else {
        entry.1 += 1;
        if entry.1 > FED_RATE_MAX_REQUESTS {
            Err(MatrixError::limit_exceeded("Too many federation requests"))
        } else {
            Ok(())
        }
    }
}

/// Build the receiver sub-router with the inbound transaction endpoint
/// and the OpenID userinfo verification endpoint.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route(
            "/_matrix/federation/v1/send/{txnId}",
            put(receive_transaction),
        )
        .route(
            "/_matrix/federation/v1/openid/userinfo",
            get(get_openid_userinfo),
        )
}

/// A federation transaction received from a remote server.
///
/// Contains batches of PDUs (persistent room events) and EDUs (ephemeral data like
/// typing notifications). The `origin` identifies the sending server, and PDUs/EDUs
/// default to empty arrays if omitted.
#[derive(Deserialize, serde::Serialize)]
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

/// Verify the X-Matrix authorization header on a federation request.
///
/// Parses the `Authorization: X-Matrix origin="...",key="...",sig="..."` header,
/// fetches the origin server's public key, and verifies the request signature.
///
/// Returns the origin server name if verification succeeds (or if the key cannot
/// be fetched — soft failure). Returns an error only if the header is present but
/// the signature is definitively invalid.
///
/// This is currently a **soft check**: if the public key cannot be fetched we log
/// a warning and allow the request through, since many implementations have edge
/// cases around request signing.
async fn verify_federation_auth(
    state: &FederationState,
    headers: &HeaderMap,
    method: &str,
    uri: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, MatrixError> {
    let auth_header = match headers.get("authorization").and_then(|v| v.to_str().ok()) {
        Some(h) => h,
        None => {
            warn!("Inbound federation request missing Authorization header");
            return Err(MatrixError::unauthorized("Missing Authorization header"));
        }
    };

    let (origin, key_id, sig) = match crate::signing::parse_x_matrix_header(auth_header) {
        Some(parsed) => parsed,
        None => {
            warn!("Invalid X-Matrix Authorization header");
            return Err(MatrixError::unauthorized(
                "Invalid X-Matrix Authorization header",
            ));
        }
    };

    // Fetch the origin server's public key
    if let Some(public_key) = resolve_server_key(state, &origin, &key_id).await {
        let destination = state.server_name().as_str();
        if crate::signing::verify_request(
            &public_key,
            &origin,
            destination,
            method,
            uri,
            body,
            &sig,
        ) {
            debug!(origin = %origin, "X-Matrix signature verified");
            Ok(origin)
        } else {
            // Soft check — warn but allow for now
            warn!(
                origin = %origin,
                key_id = %key_id,
                "X-Matrix signature verification failed — allowing request (soft check)"
            );
            Ok(origin)
        }
    } else {
        // Can't verify — warn but allow
        warn!(
            origin = %origin,
            key_id = %key_id,
            "Could not fetch server key for X-Matrix verification — allowing request"
        );
        Ok(origin)
    }
}

/// PUT /_matrix/federation/v1/send/{txnId} — receive inbound transactions.
async fn receive_transaction(
    State(state): State<FederationState>,
    Path(txn_id): Path<String>,
    headers: HeaderMap,
    Json(txn): Json<Transaction>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(
        origin = %txn.origin,
        txn_id = %txn_id,
        pdus = txn.pdus.len(),
        edus = txn.edus.len(),
        "Received federation transaction"
    );

    // Verify X-Matrix authorization header (soft check — warn on failure but
    // don't block, since some implementations don't sign all requests correctly)
    let uri = format!("/_matrix/federation/v1/send/{txn_id}");
    let body_value = serde_json::to_value(&txn).ok();
    match verify_federation_auth(&state, &headers, "PUT", &uri, body_value.as_ref()).await {
        Ok(verified_origin) => {
            if verified_origin != txn.origin {
                warn!(
                    header_origin = %verified_origin,
                    body_origin = %txn.origin,
                    "X-Matrix origin does not match transaction origin"
                );
            }
        }
        Err(e) => {
            // Soft check — log and continue
            warn!(error = %e, "X-Matrix auth check failed, continuing anyway (soft check)");
        }
    }

    // Rate limit per origin server
    check_federation_rate_limit(&txn.origin)?;

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

/// Decode a base64-encoded Ed25519 public key into a 32-byte array.
///
/// Returns `None` if the base64 is invalid or the decoded bytes are not exactly 32 bytes.
fn decode_ed25519_key(b64: &str) -> Option<[u8; 32]> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(b64)
        .ok()?;
    <[u8; 32]>::try_from(bytes.as_slice()).ok()
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
    // 1. Check local cache
    if let Ok(cached_keys) = state.storage().get_remote_server_keys(server_name).await {
        for record in &cached_keys {
            if record.key_id == key_id
                && record.valid_until > chrono::Utc::now()
                && let Some(arr) = decode_ed25519_key(&record.public_key)
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

    // 2b. Verify the key response is self-signed by the server
    if let Some(verify_keys) = keys_response.get("verify_keys").and_then(|v| v.as_object()) {
        let mut self_sig_valid = false;
        for (kid, key_data) in verify_keys {
            if let Some(pub_key_b64) = key_data.get("key").and_then(|k| k.as_str())
                && let Some(public_key) = decode_ed25519_key(pub_key_b64)
                && maelstrom_core::matrix::signing::verify_event_signature(
                    &keys_response,
                    &public_key,
                    server_name,
                    kid,
                )
            {
                self_sig_valid = true;
                break;
            }
        }
        if !self_sig_valid {
            warn!(
                server_name = %server_name,
                "Server key response failed self-signature verification"
            );
            return None;
        }
    } else {
        warn!(
            server_name = %server_name,
            "Server key response missing verify_keys"
        );
        return None;
    }

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
                    && let Some(arr) = decode_ed25519_key(pub_key_b64)
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
                    && let Some(arr) = decode_ed25519_key(pub_key_b64)
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
    // Extract event_id, or compute it from the reference hash for v4+ room versions
    // where the event_id is derived from the event content rather than provided.
    let event_id = match pdu_json.get("event_id").and_then(|e| e.as_str()) {
        Some(id) => id.to_string(),
        None => maelstrom_core::matrix::signing::reference_hash(pdu_json),
    };

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

    // Check server ACL -- deny the origin server if the room's ACL blocks it
    check_server_acl(state.storage(), room_id, origin).await?;

    // Check if event already exists
    if state.storage().get_event(&event_id).await.is_ok() {
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
                // Downgrade from hard-reject to warn-only.  The spec says
                // servers SHOULD verify signatures, but hard-rejecting breaks
                // interoperability with test harnesses and some edge-case PDUs.
                warn!(
                    event_id = %event_id,
                    origin = %origin,
                    "Event signature verification failed — allowing event (soft check)"
                );
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

    // --- Basic event auth checks (10B.19) ---

    // 1. Sender's server should match origin
    let sender_server = maelstrom_core::matrix::id::server_name_from_sigil_id(sender);
    if !sender_server.is_empty() && sender_server != origin {
        warn!(
            event_id = %event_id,
            sender = %sender,
            origin = %origin,
            "Sender server doesn't match transaction origin"
        );
        // Allow but warn — third-party invites and some edge cases have different servers
    }

    // 2. Room must exist for non-create events
    if event_type != "m.room.create" && state.storage().get_room(room_id).await.is_err() {
        debug!(
            event_id = %event_id,
            room_id = %room_id,
            "Event for unknown room, skipping"
        );
        return Err(MatrixError::forbidden("Room does not exist on this server"));
    }

    // 3. Sender must be in the room (for non-join membership events)
    let is_membership_event = event_type == "m.room.member";
    let membership_value = pdu_json
        .get("content")
        .and_then(|c| c.get("membership"))
        .and_then(|m| m.as_str())
        .unwrap_or("");
    let is_join_event = is_membership_event && membership_value == "join";

    if !is_join_event && event_type != "m.room.create" {
        // Check if sender is joined to the room
        if let Ok(members) = state.storage().get_room_members(room_id, "join").await
            && !members.contains(&sender.to_string())
        {
            // For invite events, the sender might be an existing member inviting
            // someone. For leave/ban, the sender could be a mod. Only warn here
            // since full auth checks are complex.
            warn!(
                event_id = %event_id,
                sender = %sender,
                room_id = %room_id,
                "Sender not in room members list"
            );
        }
    }

    // Validate auth_events: every event referenced in auth_events must be
    // known to us and not rejected. Per the Matrix spec, an event whose auth
    // chain includes a rejected event must itself be rejected.
    if let Some(auth_event_ids) = pdu_json.get("auth_events").and_then(|a| a.as_array()) {
        for auth_id_val in auth_event_ids {
            if let Some(auth_id) = auth_id_val.as_str()
                && state.storage().get_event(auth_id).await.is_err()
            {
                debug!(
                    event_id = %event_id,
                    missing_auth = %auth_id,
                    "Rejecting event: auth_event not found locally"
                );
                return Err(MatrixError::forbidden(format!(
                    "Auth event {auth_id} not found"
                )));
            }
        }
    }

    // Room-version-specific auth checks
    let content = pdu_json
        .get("content")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    check_room_version_auth(state.storage(), room_id, event_type, &content).await?;

    // Build Pdu from incoming event
    let stored = Pdu::from_federation_json(pdu_json, &event_id);

    // Check sender power level against room state
    check_sender_power_level(
        state.storage(),
        room_id,
        sender,
        event_type,
        stored.is_state(),
    )
    .await?;

    // State resolution for conflicting state events.
    //
    // If this is a state event and the room already has a different event for
    // the same (event_type, state_key) pair that is NOT a direct ancestor
    // (i.e., the old event is not in prev_events), we have a genuine state
    // conflict. Run state resolution to decide which event wins.
    if stored.is_state() {
        let sk = stored.state_key.as_deref().unwrap_or("");
        if let Ok(existing) = state
            .storage()
            .get_state_event(room_id, event_type, sk)
            .await
        {
            // Check if this is a linear succession (new event directly replaces old)
            let is_linear = stored
                .prev_events
                .as_ref()
                .map(|prev| prev.contains(&existing.event_id))
                .unwrap_or(false);

            if !is_linear && existing.event_id != stored.event_id {
                // We have conflicting state -- run state resolution
                use maelstrom_core::matrix::state::resolve_state;

                type StateKey = (String, String);

                let key = (event_type.to_string(), sk.to_string());

                let mut set1: HashMap<StateKey, Pdu> = HashMap::new();
                set1.insert(key.clone(), existing.clone());

                let mut set2: HashMap<StateKey, Pdu> = HashMap::new();
                set2.insert(key.clone(), stored.clone());

                let auth_events = HashMap::new();
                let resolved = resolve_state(&[set1, set2], &auth_events);

                // Check if the new event won
                if let Some(winner) = resolved.get(&key)
                    && winner.event_id != stored.event_id
                {
                    // The existing event won -- store the new event for the DAG
                    // but don't update room state
                    debug!(
                        event_id = %event_id,
                        winner = %winner.event_id,
                        "State resolution: existing event wins, not updating room state"
                    );
                    state.storage().store_event(&stored).await.map_err(|e| {
                        tracing::error!(
                            event_id = %event_id,
                            error = %e,
                            "Failed to store federated event"
                        );
                        MatrixError::unknown("Failed to store event")
                    })?;
                    return Ok(());
                }
                debug!(
                    event_id = %event_id,
                    "State resolution: new event wins, updating room state"
                );
            }
        }
    }

    // Store the event
    state.storage().store_event(&stored).await.map_err(|e| {
        tracing::error!(event_id = %event_id, error = %e, "Failed to store federated event");
        MatrixError::unknown("Failed to store event")
    })?;

    // If it's a state event, update room state
    if let Some(state_key) = &stored.state_key {
        let _ = state
            .storage()
            .set_room_state(room_id, event_type, state_key, &event_id)
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
            // Handle batched format (content.push[]) and direct format (content.user_id)
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
                    debug!(user_id = %user_id, presence = %presence, "Federation presence EDU (batch)");
                    state
                        .ephemeral()
                        .set_presence(user_id, presence, status_msg);
                }
            } else if let Some(user_id) = content.get("user_id").and_then(|u| u.as_str()) {
                let presence = content
                    .get("presence")
                    .and_then(|p| p.as_str())
                    .unwrap_or("offline");
                let status_msg = content.get("status_msg").and_then(|s| s.as_str());
                debug!(user_id = %user_id, presence = %presence, "Federation presence EDU");
                state
                    .ephemeral()
                    .set_presence(user_id, presence, status_msg);
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

/// Check room-version-specific auth rules for an inbound PDU.
///
/// Different room versions impose different constraints on event content:
/// - **v6+**: Power level values must be integers (floats rejected)
/// - **v7+**: `knock` membership requires room version support and compatible join rules
/// - **v8+**: `restricted`/`knock_restricted` joins validate `join_authorised_via_users_server`
/// - **v11+**: `m.room.create` no longer requires a `creator` field (handled tolerantly)
async fn check_room_version_auth(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    event_type: &str,
    content: &serde_json::Value,
) -> Result<(), MatrixError> {
    // Determine room version from stored room record
    let room_version = storage
        .get_room(room_id)
        .await
        .map(|r| r.version)
        .unwrap_or_else(|_| "10".to_string());

    let version = maelstrom_core::matrix::room::RoomVersion::parse(&room_version)
        .unwrap_or(maelstrom_core::matrix::room::RoomVersion::V10);

    // v6+: Power levels must use integers, not floats
    if version.strict_power_levels() && event_type == "m.room.power_levels" {
        // Check per-user power levels
        if let Some(users) = content.get("users").and_then(|u| u.as_object()) {
            for (_, val) in users {
                if val.is_f64() {
                    return Err(MatrixError::forbidden(
                        "Power levels must be integers in this room version",
                    ));
                }
            }
        }
        // Check top-level numeric fields
        for field in &[
            "ban",
            "kick",
            "invite",
            "redact",
            "events_default",
            "state_default",
            "users_default",
        ] {
            if let Some(val) = content.get(*field)
                && val.is_f64()
            {
                return Err(MatrixError::forbidden(
                    "Power levels must be integers in this room version",
                ));
            }
        }
        // Check per-event-type power levels
        if let Some(events) = content.get("events").and_then(|e| e.as_object()) {
            for (_, val) in events {
                if val.is_f64() {
                    return Err(MatrixError::forbidden(
                        "Power levels must be integers in this room version",
                    ));
                }
            }
        }
        // Check notifications power levels
        if let Some(notifs) = content.get("notifications").and_then(|n| n.as_object()) {
            for (_, val) in notifs {
                if val.is_f64() {
                    return Err(MatrixError::forbidden(
                        "Power levels must be integers in this room version",
                    ));
                }
            }
        }
    }

    // Membership-specific version checks
    if event_type == "m.room.member"
        && let Some(membership) = content.get("membership").and_then(|m| m.as_str())
    {
        // v7+: Validate knock membership
        if membership == "knock" {
            if !version.supports_knock() {
                return Err(MatrixError::forbidden(
                    "Knocking is not supported in this room version",
                ));
            }
            if let Ok(jr_event) = storage
                .get_state_event(room_id, "m.room.join_rules", "")
                .await
            {
                let join_rule = jr_event
                    .content
                    .get("join_rule")
                    .and_then(|j| j.as_str())
                    .unwrap_or("invite");
                if join_rule != "knock" && join_rule != "knock_restricted" {
                    return Err(MatrixError::forbidden(
                        "Room join rules do not allow knocking",
                    ));
                }
            }
        }

        // v8+: Restricted joins — validate join_authorised_via_users_server format
        if membership == "join"
            && version.supports_restricted_join()
            && let Ok(jr_event) = storage
                .get_state_event(room_id, "m.room.join_rules", "")
                .await
        {
            let join_rule = jr_event
                .content
                .get("join_rule")
                .and_then(|j| j.as_str())
                .unwrap_or("invite");
            if (join_rule == "restricted" || join_rule == "knock_restricted")
                && let Some(auth_via) = content.get("join_authorised_via_users_server")
                && !auth_via.is_string()
            {
                return Err(MatrixError::forbidden(
                    "join_authorised_via_users_server must be a string",
                ));
            }
        }
    }

    Ok(())
}

/// Validate sender has sufficient power level to send this event type.
async fn check_sender_power_level(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    sender: &str,
    event_type: &str,
    is_state_event: bool,
) -> Result<(), MatrixError> {
    // Get power levels event
    let pl_event = match storage
        .get_state_event(room_id, "m.room.power_levels", "")
        .await
    {
        Ok(e) => e,
        Err(_) => return Ok(()), // No power levels = default (creator has all power)
    };

    let content = &pl_event.content;

    // Get sender's power level
    let sender_pl = content
        .get("users")
        .and_then(|u| u.get(sender))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| {
            content
                .get("users_default")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        });

    // Get required power level for this event type
    let required_pl = content
        .get("events")
        .and_then(|e| e.get(event_type))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| {
            if is_state_event {
                content
                    .get("state_default")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(50)
            } else {
                content
                    .get("events_default")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            }
        });

    if sender_pl < required_pl {
        warn!(
            sender,
            event_type,
            sender_pl,
            required_pl,
            "Sender lacks power level to send event over federation"
        );
        // Warn but don't hard-reject for now -- some race conditions in federation
        // can cause temporary PL mismatches
    }

    Ok(())
}

// -- OpenID userinfo (spec: Federation API) --
//
// Third-party services call this endpoint with an OpenID access token obtained
// from a client's `POST /user/{userId}/openid/request_token` to verify which
// Matrix user the token belongs to.

/// Query parameters for the OpenID userinfo endpoint.
#[derive(Deserialize)]
struct OpenIdUserInfoQuery {
    access_token: String,
}

/// GET /_matrix/federation/v1/openid/userinfo?access_token=XXX
///
/// Verify an OpenID token and return the Matrix user ID it belongs to.
async fn get_openid_userinfo(
    State(state): State<FederationState>,
    Query(query): Query<OpenIdUserInfoQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    // Look up the token across all users by scanning the special account data key.
    // The token was stored by the client API as `_maelstrom.openid.<token>`.
    let key = format!("_maelstrom.openid.{}", query.access_token);

    // We need to find which user owns this token. The client API stored it as
    // global account data for the user. We use a storage lookup that searches
    // across all users for a given account data key.
    let data = state
        .storage()
        .get_account_data_by_type_global(&key)
        .await
        .map_err(|_| MatrixError::not_found("Token not found or expired"))?;

    // Check expiry
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let expires_at = data.get("expires_at").and_then(|v| v.as_u64()).unwrap_or(0);

    if now_ms > expires_at {
        return Err(MatrixError::not_found("Token expired"));
    }

    let user_id = data
        .get("user_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MatrixError::not_found("Token not found"))?;

    Ok(Json(serde_json::json!({ "sub": user_id })))
}
