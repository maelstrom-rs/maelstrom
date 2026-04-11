//! # Federation Join and Leave Protocol
//!
//! This module implements the most complex federation flow in Matrix: how a user on
//! one server joins (or leaves) a room that lives on another server.
//!
//! ## The Join Handshake
//!
//! Joining a room over federation is a **two-phase handshake**:
//!
//! ### Phase 1: `make_join`
//!
//! The joining server sends `GET /_matrix/federation/v1/make_join/{roomId}/{userId}`
//! to the room's resident server. The resident server returns a **join event template**
//! -- a partially filled `m.room.member` event with the correct `auth_events`,
//! `prev_events`, `depth`, and room version. This template is NOT yet signed or stored.
//!
//! ### Phase 2: `send_join`
//!
//! The joining server fills in the template, signs it with its own key, and sends it
//! back via `PUT /_matrix/federation/v2/send_join/{roomId}/{eventId}`. The resident
//! server:
//!
//! 1. Stores the signed join event
//! 2. Updates the user's membership to "join"
//! 3. Updates the room's current state
//! 4. Returns the **full room state** and **auth chain** so the joining server can
//!    bootstrap its view of the room
//!
//! ## The Leave Handshake
//!
//! Leaving follows the same pattern: `make_leave` returns a template, the departing
//! server signs it, and `send_leave` commits it. The response is simpler since the
//! leaving server does not need the room state.
//!
//! ## V1 vs V2
//!
//! Both `send_join` and `send_leave` have v1 and v2 variants. The v1 endpoints wrap
//! the response in an array `[200, {...}]` for historical reasons. The v2 endpoints
//! return the response object directly.
//!
//! ## Endpoints
//!
//! - `GET /make_join/{roomId}/{userId}` -- get a join event template
//! - `PUT /send_join/{roomId}/{eventId}` -- commit a signed join event (v1 and v2)
//! - `GET /make_leave/{roomId}/{userId}` -- get a leave event template
//! - `PUT /send_leave/{roomId}/{eventId}` -- commit a signed leave event (v1 and v2)

use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::{Pdu, timestamp_ms};
use maelstrom_core::matrix::room::Membership;
use maelstrom_core::matrix::room::event_type as et;

use crate::FederationState;

/// Build the joins sub-router with all join/leave federation endpoints.
pub fn routes() -> Router<FederationState> {
    Router::new()
        .route(
            "/_matrix/federation/v1/make_join/{roomId}/{userId}",
            get(make_join),
        )
        .route(
            "/_matrix/federation/v1/send_join/{roomId}/{eventId}",
            put(send_join_v1),
        )
        .route(
            "/_matrix/federation/v2/send_join/{roomId}/{eventId}",
            put(send_join),
        )
        .route(
            "/_matrix/federation/v1/make_leave/{roomId}/{userId}",
            get(make_leave),
        )
        .route(
            "/_matrix/federation/v1/send_leave/{roomId}/{eventId}",
            put(send_leave_v1),
        )
        .route(
            "/_matrix/federation/v2/send_leave/{roomId}/{eventId}",
            put(send_leave),
        )
}

#[derive(Deserialize)]
struct MakeJoinParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "userId")]
    user_id: String,
}

/// GET /_matrix/federation/v1/make_join/{roomId}/{userId}
/// Returns a join event template for the remote server to sign.
async fn make_join(
    State(state): State<FederationState>,
    Path(params): Path<MakeJoinParams>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, user_id = %params.user_id, "make_join request");

    // Verify room exists
    let room = state
        .storage()
        .get_room(&params.room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found on this server"))?;

    // Check server ACL for the joining user's server
    let joining_server = maelstrom_core::matrix::id::server_name_from_sigil_id(&params.user_id);
    if !joining_server.is_empty() {
        check_server_acl(state.storage(), &params.room_id, joining_server).await?;
    }

    // Get auth events for the join (create, join_rules, power_levels, current member state)
    let auth_event_ids = get_auth_event_ids(state.storage(), &params.room_id).await;

    // Get forward extremities for prev_events
    let prev_events = get_latest_event_ids(state.storage(), &params.room_id).await;

    let event_template = serde_json::json!({
        "room_id": params.room_id,
        "sender": params.user_id,
        "type": et::MEMBER,
        "state_key": params.user_id,
        "content": {
            "membership": Membership::Join.as_str(),
        },
        "origin": maelstrom_core::matrix::id::server_name_from_sigil_id(&params.user_id),
        "origin_server_ts": timestamp_ms(),
        "auth_events": auth_event_ids,
        "prev_events": prev_events,
        "depth": 100, // Simplified; real depth would be max(prev_events depths) + 1
    });

    Ok(Json(serde_json::json!({
        "event": event_template,
        "room_version": room.version,
    })))
}

#[derive(Deserialize)]
struct SendJoinParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

/// Query parameters for send_join (MSC3706 partial state support).
#[derive(Deserialize, Default)]
struct SendJoinQuery {
    /// When true, the joining server requests partial state (MSC3706 "faster joins").
    #[serde(default, rename = "org.matrix.msc3706.partial_state")]
    partial_state: bool,
}

/// PUT /_matrix/federation/v2/send_join/{roomId}/{eventId}
/// Accept a signed join event from a remote server.
async fn send_join(
    State(state): State<FederationState>,
    Path(params): Path<SendJoinParams>,
    axum::extract::Query(query): axum::extract::Query<SendJoinQuery>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(
        room_id = %params.room_id,
        event_id = %params.event_id,
        partial_state = query.partial_state,
        "send_join request"
    );

    let sender = event_json
        .get("sender")
        .and_then(|s| s.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing sender"))?
        .to_string();

    // Check server ACL for the joining user's server
    let joining_server = maelstrom_core::matrix::id::server_name_from_sigil_id(&sender);
    if !joining_server.is_empty() {
        check_server_acl(state.storage(), &params.room_id, joining_server).await?;
    }

    let event_type = event_json
        .get("type")
        .and_then(|s| s.as_str())
        .unwrap_or(et::MEMBER);

    // Store the join event
    let stored = Pdu {
        event_id: params.event_id.clone(),
        room_id: params.room_id.clone(),
        sender: sender.clone(),
        event_type: event_type.to_string(),
        state_key: event_json
            .get("state_key")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        content: event_json
            .get("content")
            .cloned()
            .unwrap_or(serde_json::json!({})),
        origin_server_ts: event_json
            .get("origin_server_ts")
            .and_then(|t| t.as_u64())
            .unwrap_or(0),
        unsigned: None,
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

    state.storage().store_event(&stored).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to store join event");
        MatrixError::unknown("Failed to store event")
    })?;

    // Update membership
    state
        .storage()
        .set_membership(&sender, &params.room_id, Membership::Join.as_str())
        .await
        .map_err(|_| MatrixError::unknown("Failed to update membership"))?;

    // Update room state
    if let Some(state_key) = &stored.state_key {
        let _ = state
            .storage()
            .set_room_state(&params.room_id, event_type, state_key, &params.event_id)
            .await;
    }

    // Fetch current room state and compute auth chain
    let current_state = state
        .storage()
        .get_current_state(&params.room_id)
        .await
        .unwrap_or_default();

    let auth_chain = compute_auth_chain(state.storage(), &current_state).await;

    // MSC3706: when partial_state is requested, only return auth-related state
    // events (create, power_levels, join_rules, the joining user's member event)
    // instead of the full room state. The joining server will fetch the rest
    // via /state_ids and /state in the background.
    let state_events: Vec<serde_json::Value> = if query.partial_state {
        current_state
            .iter()
            .filter(|e| {
                e.event_type == et::CREATE
                    || e.event_type == et::POWER_LEVELS
                    || e.event_type == et::JOIN_RULES
                    || e.event_type == et::HISTORY_VISIBILITY
                    || (e.event_type == et::MEMBER && e.state_key.as_deref() == Some(&sender))
            })
            .map(|e| e.to_federation_json())
            .collect()
    } else {
        current_state
            .iter()
            .map(|e| e.to_federation_json())
            .collect()
    };

    let mut response = serde_json::json!({
        "origin": state.server_name().as_str(),
        "state": state_events,
        "auth_chain": auth_chain,
        "event": event_json,
    });

    // MSC3706: include additional fields when partial state was requested
    if query.partial_state {
        // Confirm partial state was used
        response["org.matrix.msc3706.partial_state"] = serde_json::json!(true);
        // Signal that member events were omitted from state
        response["members_omitted"] = serde_json::json!(true);
        // Provide the list of servers currently in the room so the joining
        // server knows who to contact for missing state and federation
        let servers = servers_in_room(state.storage(), &params.room_id).await;
        response["servers_in_room"] = serde_json::json!(servers);
    }

    Ok(Json(response))
}

/// PUT /_matrix/federation/v1/send_join — v1 returns [200, { ... }]
async fn send_join_v1(
    State(state): State<FederationState>,
    Path(params): Path<SendJoinParams>,
    query: axum::extract::Query<SendJoinQuery>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let result = send_join(State(state), Path(params), query, Json(event_json)).await?;
    Ok(Json(serde_json::json!([200, result.0])))
}

#[derive(Deserialize)]
struct MakeLeaveParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "userId")]
    user_id: String,
}

/// GET /_matrix/federation/v1/make_leave/{roomId}/{userId}
async fn make_leave(
    State(state): State<FederationState>,
    Path(params): Path<MakeLeaveParams>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, user_id = %params.user_id, "make_leave request");

    let room = state
        .storage()
        .get_room(&params.room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let auth_event_ids = get_auth_event_ids(state.storage(), &params.room_id).await;
    let prev_events = get_latest_event_ids(state.storage(), &params.room_id).await;

    let event_template = serde_json::json!({
        "room_id": params.room_id,
        "sender": params.user_id,
        "type": et::MEMBER,
        "state_key": params.user_id,
        "content": {
            "membership": Membership::Leave.as_str(),
        },
        "origin": maelstrom_core::matrix::id::server_name_from_sigil_id(&params.user_id),
        "origin_server_ts": timestamp_ms(),
        "auth_events": auth_event_ids,
        "prev_events": prev_events,
        "depth": 100,
    });

    Ok(Json(serde_json::json!({
        "event": event_template,
        "room_version": room.version,
    })))
}

#[derive(Deserialize)]
struct SendLeaveParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

/// PUT /_matrix/federation/v2/send_leave/{roomId}/{eventId}
async fn send_leave(
    State(state): State<FederationState>,
    Path(params): Path<SendLeaveParams>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    debug!(room_id = %params.room_id, event_id = %params.event_id, "send_leave request");

    let sender = event_json
        .get("sender")
        .and_then(|s| s.as_str())
        .ok_or_else(|| MatrixError::bad_json("Missing sender"))?
        .to_string();

    // Store the leave event
    let stored = Pdu {
        event_id: params.event_id.clone(),
        room_id: params.room_id.clone(),
        sender: sender.clone(),
        event_type: et::MEMBER.to_string(),
        state_key: Some(sender.clone()),
        content: serde_json::json!({ "membership": Membership::Leave.as_str() }),
        origin_server_ts: event_json
            .get("origin_server_ts")
            .and_then(|t| t.as_u64())
            .unwrap_or(0),
        unsigned: None,
        stream_position: 0,
        origin: event_json
            .get("origin")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string()),
        auth_events: None,
        prev_events: None,
        depth: event_json.get("depth").and_then(|d| d.as_i64()),
        hashes: event_json.get("hashes").cloned(),
        signatures: event_json.get("signatures").cloned(),
    };

    let _ = state.storage().store_event(&stored).await;

    // Update membership
    let _ = state
        .storage()
        .set_membership(&sender, &params.room_id, Membership::Leave.as_str())
        .await;

    // Update room state
    let _ = state
        .storage()
        .set_room_state(&params.room_id, et::MEMBER, &sender, &params.event_id)
        .await;

    Ok(Json(serde_json::json!({})))
}

/// PUT /_matrix/federation/v1/send_leave — v1 returns [200, {}]
async fn send_leave_v1(
    State(state): State<FederationState>,
    Path(params): Path<SendLeaveParams>,
    Json(event_json): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let result = send_leave(State(state), Path(params), Json(event_json)).await?;
    Ok(Json(serde_json::json!([200, result.0])))
}

/// Collect the set of server names with joined members in a room.
///
/// Used by MSC3706 partial state joins to tell the joining server which
/// other servers are participating, so it can contact them for missing
/// state and ongoing federation.
async fn servers_in_room(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
) -> Vec<String> {
    let mut servers = std::collections::HashSet::new();
    if let Ok(members) = storage
        .get_room_members(room_id, Membership::Join.as_str())
        .await
    {
        for member in members {
            let server = maelstrom_core::matrix::id::server_name_from_sigil_id(&member);
            if !server.is_empty() {
                servers.insert(server.to_string());
            }
        }
    }
    servers.into_iter().collect()
}

/// Compute the auth chain for a set of state events.
///
/// Returns the transitive closure of all events referenced by `auth_events` fields,
/// starting from the given state events and walking the graph until no new events
/// are found. This is the minimal set of events needed to verify the state per
/// section 10B.20 of the federation spec.
pub(crate) async fn compute_auth_chain(
    storage: &dyn maelstrom_storage::traits::Storage,
    state_events: &[Pdu],
) -> Vec<serde_json::Value> {
    let mut seen = std::collections::HashSet::new();
    let mut queue: Vec<String> = Vec::new();
    let mut chain = Vec::new();

    // Seed with auth_events from all state events
    for event in state_events {
        if let Some(auth_ids) = &event.auth_events {
            for id in auth_ids {
                if !seen.contains(id) {
                    seen.insert(id.clone());
                    queue.push(id.clone());
                }
            }
        }
    }

    // BFS through auth_events graph
    while let Some(event_id) = queue.pop() {
        if let Ok(event) = storage.get_event(&event_id).await {
            chain.push(event.to_federation_json());
            if let Some(auth_ids) = &event.auth_events {
                for id in auth_ids {
                    if !seen.contains(id) {
                        seen.insert(id.clone());
                        queue.push(id.clone());
                    }
                }
            }
        }
    }

    chain
}

/// Get auth event IDs for a room (create, join_rules, power_levels events).
///
/// Auth events are the minimal set of state events needed to authorize a new event.
/// For membership events, this includes the room creation event, join rules, and
/// power levels. These IDs are included in the `auth_events` field of event templates
/// so the joining server can verify the event is allowed.
async fn get_auth_event_ids(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
) -> Vec<String> {
    let mut ids = Vec::new();
    for (event_type, state_key) in [
        (et::CREATE, ""),
        (et::JOIN_RULES, ""),
        (et::POWER_LEVELS, ""),
    ] {
        if let Ok(event) = storage
            .get_state_event(room_id, event_type, state_key)
            .await
        {
            ids.push(event.event_id);
        }
    }
    ids
}

/// Get the latest event IDs in a room (forward extremities).
///
/// In a full implementation, these would be the true DAG leaf nodes. Currently
/// simplified to return the last 2 events by stream position. These become the
/// `prev_events` in the event template, linking the new event into the room's DAG.
async fn get_latest_event_ids(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
) -> Vec<String> {
    if let Ok(pos) = storage.current_stream_position().await
        && let Ok(events) = storage.get_room_events(room_id, pos + 1, 2, "b").await
    {
        return events.into_iter().map(|e| e.event_id).collect();
    }
    Vec::new()
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
