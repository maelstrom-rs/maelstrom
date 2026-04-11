//! # Federation Invite Flow
//!
//! When a user on server A invites a user on server B to a room, the invite must
//! be delivered over federation. This module handles the **receiving** side -- when
//! a remote server sends us an invite for one of our local users.
//!
//! ## How Federation Invites Work
//!
//! 1. The inviting server constructs an `m.room.member` event with `membership: invite`
//!    and the invited user's ID as the `state_key`.
//! 2. The inviting server sends this event to the invited user's server via
//!    `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}`.
//! 3. The receiving server (this code):
//!    - Verifies the invited user belongs to this server
//!    - Creates the room locally if it does not already exist (since the invited user
//!      may not have seen this room before)
//!    - Stores the invite event
//!    - Sets the user's membership to "invite" in that room
//!    - Returns the event (potentially with this server's signature added)
//!
//! ## V1 vs V2
//!
//! - **V2** (`PUT /v2/invite/{roomId}/{eventId}`): the event is wrapped in
//!   `{"event": {...}}`. The response is `{"event": {...}}`.
//! - **V1** (`PUT /v1/invite/{roomId}/{eventId}`): the event IS the request body
//!   (unwrapped). The response is `[200, {...}]` (array with status code).
//!
//! ## Endpoints
//!
//! - `PUT /_matrix/federation/v2/invite/{roomId}/{eventId}` -- receive invite (v2 format)
//! - `PUT /_matrix/federation/v1/invite/{roomId}/{eventId}` -- receive invite (v1 format)

use axum::extract::{Path, State};
use axum::routing::{post, put};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::{debug, warn};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::{self, Pdu};
use maelstrom_core::matrix::room::Membership;
use maelstrom_core::matrix::room::event_type as et;

use crate::FederationState;

/// Build the invite sub-router with v1 and v2 invite endpoints, plus third-party
/// invite exchange.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route(
            "/_matrix/federation/v2/invite/{roomId}/{eventId}",
            put(receive_invite_v2),
        )
        .route(
            "/_matrix/federation/v1/invite/{roomId}/{eventId}",
            put(receive_invite_v1),
        )
        .route(
            "/_matrix/federation/v2/exchange_third_party_invite/{roomId}",
            post(exchange_third_party_invite),
        )
}

#[derive(Deserialize)]
struct InviteParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

/// PUT /_matrix/federation/v2/invite/{roomId}/{eventId}
/// Accept an invite from a remote server for a local user.
async fn receive_invite_v2(
    State(state): State<FederationState>,
    Path(params): Path<InviteParams>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "Received federation invite v2");

    let event_json = body.get("event").unwrap_or(&body);
    let invite_event = store_invite_event(&state, &params, event_json).await?;

    Ok(Json(serde_json::json!({
        "event": invite_event,
    })))
}

/// PUT /_matrix/federation/v1/invite/{roomId}/{eventId}
/// Accept an invite (v1 format — event is the body itself, not wrapped in {"event": ...}).
async fn receive_invite_v1(
    State(state): State<FederationState>,
    Path(params): Path<InviteParams>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<(http::StatusCode, Json<serde_json::Value>), MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "Received federation invite v1");

    let invite_event = store_invite_event(&state, &params, &event_json).await?;

    // v1 returns [200, {...}] (array with status code and event)
    Ok((
        http::StatusCode::OK,
        Json(serde_json::json!([200, invite_event])),
    ))
}

/// Shared logic for processing an inbound federation invite (used by both v1 and v2).
///
/// Validates the invited user belongs to this server, creates the room locally if
/// needed, stores the invite event, sets membership to "invite", and updates room state.
async fn store_invite_event(
    state: &FederationState,
    params: &InviteParams,
    event_json: &serde_json::Value,
) -> Result<serde_json::Value, MatrixError> {
    let sender = event_json
        .get("sender")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let state_key = event_json
        .get("state_key")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    // The state_key is the invited user
    if state_key.is_empty() {
        return Err(MatrixError::bad_json("Missing state_key (invited user)"));
    }

    // Verify the invited user belongs to our server
    let user_server = maelstrom_core::matrix::id::server_name_from_sigil_id(&state_key);
    if user_server != state.server_name().as_str() {
        return Err(MatrixError::forbidden("Invited user is not on this server"));
    }

    // Check server ACL for the sender's server (if room already exists locally)
    let sender_server = maelstrom_core::matrix::id::server_name_from_sigil_id(&sender);
    if !sender_server.is_empty() {
        check_server_acl(state.storage(), &params.room_id, sender_server).await?;
    }

    // Create room locally if it doesn't exist (we only know about it through the invite)
    let room_version = event_json
        .get("room_version")
        .and_then(|v| v.as_str())
        .unwrap_or("10")
        .to_string();

    let room_record = maelstrom_storage::traits::RoomRecord {
        room_id: params.room_id.clone(),
        version: room_version,
        creator: sender.clone(),
        is_direct: event_json
            .get("content")
            .and_then(|c| c.get("is_direct"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    let _ = state.storage().create_room(&room_record).await;

    // Store the invite event
    let stored = Pdu {
        event_id: params.event_id.clone(),
        room_id: params.room_id.clone(),
        sender,
        event_type: et::MEMBER.to_string(),
        state_key: Some(state_key.clone()),
        content: event_json
            .get("content")
            .cloned()
            .unwrap_or(serde_json::json!({"membership": Membership::Invite.as_str()})),
        origin_server_ts: event_json
            .get("origin_server_ts")
            .and_then(|t| t.as_u64())
            .unwrap_or(0),
        unsigned: event_json.get("unsigned").cloned(),
        stream_position: 0,
        origin: event_json
            .get("origin")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        auth_events: event_json.get("auth_events").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        prev_events: event_json.get("prev_events").and_then(|a| {
            a.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        depth: event_json.get("depth").and_then(|d| d.as_i64()),
        hashes: event_json.get("hashes").cloned(),
        signatures: event_json.get("signatures").cloned(),
    };

    let _ = state.storage().store_event(&stored).await;

    // Set membership to invite
    state
        .storage()
        .set_membership(&state_key, &params.room_id, Membership::Invite.as_str())
        .await
        .map_err(|_| MatrixError::unknown("Failed to set invite membership"))?;

    // Update room state
    let _ = state
        .storage()
        .set_room_state(&params.room_id, et::MEMBER, &state_key, &params.event_id)
        .await;

    // Return the event (potentially with our signature added)
    Ok(event_json.clone())
}

/// POST /_matrix/federation/v2/exchange_third_party_invite/{roomId}
///
/// Converts a third-party invite into a real member event. When a user who was
/// invited by email/phone registers and their identity server confirms the
/// binding, this endpoint is called to add them to the room.
async fn exchange_third_party_invite(
    State(state): State<FederationState>,
    Path(room_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %room_id, "Exchange third-party invite");

    let sender = body
        .get("sender")
        .and_then(|s| s.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing sender"))?;
    let event_type = body
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or(et::MEMBER);
    let state_key = body
        .get("state_key")
        .and_then(|s| s.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing state_key"))?;
    let content = body
        .get("content")
        .cloned()
        .unwrap_or(serde_json::json!({"membership": Membership::Join.as_str()}));

    // Validate third-party invite token signature if present
    if let Some(tpi) = content.get("third_party_invite")
        && let Some(signed) = tpi.get("signed")
    {
        let token = signed.get("token").and_then(|t| t.as_str()).unwrap_or("");

        if let Ok(tpi_event) = state
            .storage()
            .get_state_event(&room_id, "m.room.third_party_invite", token)
            .await
            && let Some(public_keys) = tpi_event
                .content
                .get("public_keys")
                .and_then(|pk| pk.as_array())
        {
            let verified = verify_3pi_signature(signed, public_keys);
            if !verified {
                warn!(room_id = %room_id, sender = %sender, "Third-party invite signature verification failed");
                return Err(MatrixError::forbidden(
                    "Third-party invite signature verification failed",
                ));
            }
        }
    }
    let event_id = event::generate_event_id();
    let pdu = Pdu {
        event_id: event_id.clone(),
        room_id: room_id.clone(),
        sender: sender.to_string(),
        event_type: event_type.to_string(),
        state_key: Some(state_key.to_string()),
        content,
        origin_server_ts: event::timestamp_ms(),
        unsigned: None,
        stream_position: 0,
        origin: body
            .get("origin")
            .and_then(|o| o.as_str())
            .map(String::from),
        auth_events: None,
        prev_events: None,
        depth: None,
        hashes: None,
        signatures: None,
    };

    let _ = state.storage().store_event(&pdu).await;

    if event_type == et::MEMBER {
        let membership = pdu
            .content
            .get("membership")
            .and_then(|m| m.as_str())
            .unwrap_or(Membership::Join.as_str());
        let _ = state
            .storage()
            .set_membership(state_key, &room_id, membership)
            .await;
        let _ = state
            .storage()
            .set_room_state(&room_id, event_type, state_key, &event_id)
            .await;
    }

    Ok(Json(serde_json::json!({})))
}

/// Verify that a third-party invite's `signed` object has a valid Ed25519 signature
/// from one of the public keys in the original `m.room.third_party_invite` event.
fn verify_3pi_signature(signed: &serde_json::Value, public_keys: &[serde_json::Value]) -> bool {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD_NO_PAD;

    // Build canonical content to verify (signed object without the signatures field)
    let mut to_verify = signed.clone();
    if let Some(obj) = to_verify.as_object_mut() {
        obj.remove("signatures");
    }
    let canonical = serde_json::to_string(&to_verify).unwrap_or_default();

    let sigs = match signed.get("signatures").and_then(|s| s.as_object()) {
        Some(s) => s,
        None => return false,
    };

    for pk_entry in public_keys {
        let pk_b64 = match pk_entry.get("public_key").and_then(|k| k.as_str()) {
            Some(k) => k,
            None => continue,
        };
        let pk_arr: [u8; 32] = match engine
            .decode(pk_b64)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        {
            Some(a) => a,
            None => continue,
        };

        for (_server, server_sigs) in sigs {
            if let Some(server_sigs) = server_sigs.as_object() {
                for (_kid, sig_val) in server_sigs {
                    if let Some(sig_b64) = sig_val.as_str()
                        && maelstrom_core::matrix::keys::verify_signature(
                            &pk_arr,
                            canonical.as_bytes(),
                            sig_b64,
                        )
                    {
                        return true;
                    }
                }
            }
        }
    }

    false
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
