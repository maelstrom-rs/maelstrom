//! Event operations -- send, retrieve, paginate, state management, and redaction.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 9 Events](https://spec.matrix.org/v1.13/client-server-api/#events-2)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `PUT`  | `/rooms/{roomId}/send/{eventType}/{txnId}` | Send a message event |
//! | `GET`  | `/rooms/{roomId}/event/{eventId}` | Fetch a single event |
//! | `GET`  | `/rooms/{roomId}/messages` | Paginate room timeline |
//! | `PUT`  | `/rooms/{roomId}/state/{eventType}/{stateKey}` | Set state event |
//! | `GET`  | `/rooms/{roomId}/state/{eventType}/{stateKey}` | Get state event |
//! | `GET`  | `/rooms/{roomId}/state` | Get all current state |
//! | `PUT`  | `/rooms/{roomId}/redact/{eventId}/{txnId}` | Redact an event |
//!
//! All endpoints are registered under both `/_matrix/client/v3` and the legacy
//! `/_matrix/client/r0` prefix for Complement test compatibility.
//!
//! # Sending events
//!
//! **Message events** (`PUT /send`) are non-state events like `m.room.message`.
//! The `{txnId}` path parameter enables client-side deduplication: if the same
//! `(device_id, room_id, txn_id)` triple is seen again, the server returns the
//! original `event_id` without storing a duplicate. Event content is validated
//! for JSON correctness and capped at 65 KB.
//!
//! **State events** (`PUT /state`) have an additional `state_key` that
//! distinguishes multiple events of the same type (e.g., per-user member
//! events). Power-level checks are enforced: the sender's PL must meet or
//! exceed the required PL from `m.room.power_levels.events[event_type]` or
//! `state_default`. Users can always update their own `m.room.member` event
//! (for profile changes). State events with identical content are idempotent.
//!
//! When a state event is written, `m.room.canonical_alias` content is validated
//! to ensure referenced aliases actually exist and point to the correct room.
//!
//! # Retrieving events
//!
//! **Single event** (`GET /event`) returns one event by ID. Access is governed
//! by the room's `m.room.history_visibility` setting:
//! - `world_readable` -- anyone can see events
//! - `shared` -- any current or former member
//! - `invited` -- must have been at least invited at the event's stream position
//! - `joined` -- must have been joined at the event's stream position
//!
//! **Messages** (`GET /messages`) paginates the room timeline forward (`dir=f`)
//! or backward (`dir=b`) from a stream-position token. Supports `lazy_load_members`
//! and `related_by_rel_types` (MSC3874) filters. For departed users, events are
//! capped at the stream position of their leave event.
//!
//! **Full state** (`GET /state`) returns all current state events. For departed
//! users, state is frozen at the point they left.
//!
//! # Redaction
//!
//! `PUT /redact` creates an `m.room.redaction` event and then strips the
//! target event's content via `storage.redact_event()`. The redaction event
//! itself is persisted in the timeline so other users see it in sync. Transaction
//! ID deduplication applies to redactions the same as message sends.
//!
//! # Relations
//!
//! After storing a message event, [`extract_and_store_relation`] inspects the
//! `m.relates_to` content field for relation metadata (threads, reactions,
//! edits) and persists it as a separate relation record for efficient lookup.

use axum::extract::{Path, Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::{Pdu, generate_event_id, timestamp_ms};
use maelstrom_core::matrix::id::server_name_from_sigil_id;
use maelstrom_core::matrix::room::event_type as et;
use maelstrom_core::matrix::room::{HistoryVisibility, Membership};
use maelstrom_storage::traits::StorageError;
use tracing::warn;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::handlers::util::require_membership;
use crate::notify::Notification;
use crate::state::AppState;

/// Register all event operation routes.
///
/// Builds routes under both `/_matrix/client/v3` and `/_matrix/client/r0`
/// prefixes for backward compatibility with older clients and the Complement
/// test harness. Includes trailing-slash variants for state endpoints.
pub fn routes() -> Router<AppState> {
    // Build routes for both v3 and r0 (Complement uses both)
    let mut router = Router::new();

    for prefix in ["/_matrix/client/v3", "/_matrix/client/r0"] {
        router = router
            .route(
                &format!("{prefix}/rooms/{{roomId}}/send/{{eventType}}/{{txnId}}"),
                put(send_event),
            )
            .route(
                &format!("{prefix}/rooms/{{roomId}}/event/{{eventId}}"),
                get(get_event),
            )
            .route(
                &format!("{prefix}/rooms/{{roomId}}/messages"),
                get(get_messages),
            )
            .route(
                &format!("{prefix}/rooms/{{roomId}}/state/{{eventType}}/{{stateKey}}"),
                put(set_state_event).get(get_state_event),
            )
            .route(
                &format!("{prefix}/rooms/{{roomId}}/state/{{eventType}}"),
                put(set_state_event_no_key).get(get_state_event_no_key),
            )
            // Trailing-slash variants (Complement sends state requests with trailing slash)
            .route(
                &format!("{prefix}/rooms/{{roomId}}/state/{{eventType}}/"),
                put(set_state_event_no_key).get(get_state_event_no_key),
            )
            .route(
                &format!("{prefix}/rooms/{{roomId}}/state"),
                get(get_full_state),
            )
            .route(
                &format!("{prefix}/rooms/{{roomId}}/redact/{{eventId}}/{{txnId}}"),
                put(redact_event),
            );
    }

    router
}

// -- PUT /rooms/{roomId}/send/{eventType}/{txnId} --

/// Response for send, set-state, and redact operations.
///
/// Contains the `event_id` of the newly created (or deduplicated) event.
#[derive(Serialize)]
struct SendEventResponse {
    event_id: String,
}

async fn send_event(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, event_type, txn_id)): Path<(String, String, String)>,
    MatrixJson(content): MatrixJson<serde_json::Value>,
) -> Result<Json<SendEventResponse>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();
    let device_id = auth.device_id.to_string();

    // Check txn_id dedup
    if let Ok(Some(existing_event_id)) = storage.get_txn_event(&device_id, &room_id, &txn_id).await
    {
        return Ok(Json(SendEventResponse {
            event_id: existing_event_id,
        }));
    }

    // Check user is joined
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Validate content is re-serializable (catches NaN, Infinity, etc.)
    let content_str = serde_json::to_string(&content).map_err(|e| {
        MatrixError::bad_json(format!("Event content contains invalid JSON values: {e}"))
    })?;

    // Reject oversized events (spec: ~65KB limit)
    if content_str.len() > 65536 {
        return Err(MatrixError::too_large("Event content too large"));
    }

    // Create event
    let event_id = generate_event_id();
    let auth_events =
        crate::handlers::util::select_auth_events(storage, &room_id, &sender, &event_type).await;
    let event = Pdu {
        event_id: event_id.clone(),
        room_id: room_id.clone(),
        sender,
        event_type,
        state_key: None,
        content,
        origin_server_ts: timestamp_ms(),
        unsigned: Some(serde_json::json!({ "transaction_id": txn_id })),
        stream_position: 0,
        origin: None,
        auth_events: if auth_events.is_empty() {
            None
        } else {
            Some(auth_events)
        },
        prev_events: None,
        depth: None,
        hashes: None,
        signatures: None,
    };

    storage.store_event(&event).await.map_err(|e| {
        // If storage rejects the event (e.g. invalid content for SurrealDB),
        // return 400 instead of 500
        tracing::warn!(event_id = %event_id, error = %e, "Failed to store event");
        MatrixError::bad_json(format!("Failed to store event: {e}"))
    })?;

    // Extract and store relations (m.relates_to in content)
    extract_and_store_relation(storage, &event).await;

    // Store txn_id mapping
    storage
        .store_txn_id(&device_id, &room_id, &txn_id, &event_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(SendEventResponse { event_id }))
}

// -- GET /rooms/{roomId}/event/{eventId} --

async fn get_event(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, event_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check history_visibility for this room
    let history_visibility = storage
        .get_state_event(&room_id, et::HISTORY_VISIBILITY, "")
        .await
        .ok()
        .and_then(|ev| {
            ev.content
                .get("history_visibility")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| HistoryVisibility::Shared.as_str().to_string());

    // Fetch the event first
    let event = storage.get_event(&event_id).await.map_err(|e| match e {
        StorageError::NotFound => MatrixError::not_found("Event not found"),
        other => crate::extractors::storage_error(other),
    })?;

    if event.room_id != room_id {
        return Err(MatrixError::not_found("Event not found"));
    }

    // Check user membership
    let membership = storage.get_membership(&sender, &room_id).await;
    let is_joined = membership
        .as_deref()
        .map(|m| m == Membership::Join.as_str())
        .unwrap_or(false);

    // world_readable: anyone can see events
    if history_visibility == HistoryVisibility::WorldReadable.as_str() {
        let m = membership.as_deref().unwrap_or(Membership::Leave.as_str());
        return Ok(Json(event.to_client_event().with_membership(m).into_json()));
    }

    // Not world_readable — must be a current or former member
    if !is_joined && membership.is_err() {
        // Never been a member — return 404 to not reveal room existence
        return Err(MatrixError::not_found("Event not found"));
    }

    // For "joined" visibility, the user must have been joined when the event was sent
    if history_visibility == HistoryVisibility::Joined.as_str() {
        // Get the user's join event to see when they joined
        let join_event = storage
            .get_state_event(&room_id, et::MEMBER, &sender)
            .await
            .ok();

        let user_joined_at = join_event
            .as_ref()
            .filter(|e| {
                e.content.get("membership").and_then(|m| m.as_str())
                    == Some(Membership::Join.as_str())
            })
            .map(|e| e.stream_position)
            .unwrap_or(i64::MAX);

        if event.stream_position < user_joined_at {
            return Err(MatrixError::not_found("Event not found"));
        }
    }

    // For "invited" visibility, user must have been at least invited at the time of the event.
    if history_visibility == HistoryVisibility::Invited.as_str() {
        // Check if the user had an invite or join membership at the event's stream position
        let membership_at_event = storage
            .get_state_event_at(&room_id, et::MEMBER, &sender, event.stream_position)
            .await
            .ok()
            .and_then(|e| {
                e.content
                    .get("membership")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
            });

        match membership_at_event.as_deref() {
            Some(m) if m == Membership::Join.as_str() || m == Membership::Invite.as_str() => {
                // User was invited or joined at the time — allow access
            }
            _ => {
                // User wasn't yet invited at the time of this event
                return Err(MatrixError::not_found("Event not found"));
            }
        }
    }

    // "shared" visibility: any current or former member can see
    let m = membership.as_deref().unwrap_or(Membership::Leave.as_str());
    Ok(Json(event.to_client_event().with_membership(m).into_json()))
}

// -- GET /rooms/{roomId}/messages --

/// Query parameters for `GET /rooms/{roomId}/messages`.
///
/// - `from` / `to`: stream-position pagination tokens
/// - `dir`: `"b"` for backward (newest-first, default) or `"f"` for forward
/// - `limit`: max events to return (capped at 100)
/// - `filter`: optional JSON filter supporting `lazy_load_members` and
///   `related_by_rel_types`
#[derive(Deserialize)]
#[allow(dead_code)]
struct MessagesQuery {
    from: Option<String>,
    to: Option<String>,
    dir: Option<String>,
    limit: Option<usize>,
    filter: Option<String>,
}

/// Response for `GET /rooms/{roomId}/messages`.
///
/// `chunk` contains the paginated events. `start` and `end` are stream-position
/// tokens for continued pagination. `state` is present when `lazy_load_members`
/// is enabled and contains `m.room.member` events for senders in the chunk.
#[derive(Serialize)]
struct MessagesResponse {
    chunk: Vec<serde_json::Value>,
    start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<Vec<serde_json::Value>>,
}

async fn get_messages(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> Result<Json<MessagesResponse>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is a member (or was a member)
    let membership = require_membership(storage, &sender, &room_id).await?;

    let dir = query.dir.as_deref().unwrap_or("b");
    let limit = query.limit.unwrap_or(10).min(100);
    let from: i64 = match query.from.as_deref().and_then(|s| s.parse().ok()) {
        Some(pos) => pos,
        None => {
            // No from token: for backward, use current max position; for forward, use 0
            if dir == "b" {
                storage.current_stream_position().await.unwrap_or(0) + 1
            } else {
                0
            }
        }
    };

    // For departed users: limit messages to events up to when they left/were banned
    let leave_pos =
        if membership == Membership::Leave.as_str() || membership == Membership::Ban.as_str() {
            storage
                .get_state_event(&room_id, et::MEMBER, &sender)
                .await
                .ok()
                .map(|e| e.stream_position)
        } else {
            None
        };

    let mut events = storage
        .get_room_events(&room_id, from, limit, dir)
        .await
        .map_err(crate::extractors::storage_error)?;

    // If going backward and we got fewer than requested, try federation backfill
    if dir == "b"
        && events.len() < limit
        && let Some(fed_client) = state.federation()
    {
        let origin_server = server_name_from_sigil_id(&room_id);
        if !origin_server.is_empty() && origin_server != state.server_name().as_str() {
            // Use the earliest local event as anchor, or fall back to the
            // room creation event when the page is empty.
            let earliest_event_id = if let Some(last) = events.last() {
                Some(last.event_id.clone())
            } else {
                storage
                    .get_state_event(&room_id, et::CREATE, "")
                    .await
                    .ok()
                    .map(|e| e.event_id)
            };

            if let Some(earliest_id) = earliest_event_id {
                let remaining = limit - events.len();
                let path = format!(
                    "/_matrix/federation/v1/backfill/{}?limit={}&v={}",
                    crate::handlers::util::percent_encode(&room_id),
                    remaining,
                    crate::handlers::util::percent_encode(&earliest_id),
                );

                match fed_client.get(origin_server, &path).await {
                    Ok(response) => {
                        if let Some(pdus) = response.get("pdus").and_then(|p| p.as_array()) {
                            for pdu_json in pdus {
                                let event_id = pdu_json
                                    .get("event_id")
                                    .and_then(|e| e.as_str())
                                    .unwrap_or_default();

                                // Skip events we already have
                                if event_id.is_empty() || storage.get_event(event_id).await.is_ok()
                                {
                                    continue;
                                }

                                let stored = Pdu::from_federation_json(pdu_json, event_id);
                                let _ = storage.store_backfill_event(&stored).await;
                            }

                            // Re-query to include newly stored events
                            events = storage
                                .get_room_events(&room_id, from, limit, dir)
                                .await
                                .map_err(crate::extractors::storage_error)?;
                        }
                    }
                    Err(e) => {
                        warn!(
                            room_id = %room_id,
                            origin = %origin_server,
                            error = %e,
                            "Federation backfill request failed"
                        );
                    }
                }
            }
        }
    }
    // Filter out events after the user left
    let events: Vec<_> = if let Some(lp) = leave_pos {
        events
            .into_iter()
            .filter(|e| e.stream_position <= lp)
            .collect()
    } else {
        events
    };

    let start = query.from.unwrap_or_else(|| from.to_string());
    let end = events.last().map(|e| e.stream_position.to_string());

    // Parse filter JSON
    let filter_json: Option<serde_json::Value> = query
        .filter
        .as_deref()
        .and_then(|f| serde_json::from_str(f).ok());

    let lazy_load = filter_json
        .as_ref()
        .and_then(|f| f.get("lazy_load_members")?.as_bool())
        .unwrap_or(false);

    // Check for related_by_rel_types filter (MSC3874)
    let rel_type_filter: Option<Vec<String>> = filter_json
        .as_ref()
        .and_then(|f| f.get("related_by_rel_types"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });

    // If rel_type filter is set, build a set of event IDs that have matching relations
    let related_event_ids: Option<std::collections::HashSet<String>> =
        if let Some(ref rel_types) = rel_type_filter {
            let mut ids = std::collections::HashSet::new();
            for event in &events {
                for rel_type in rel_types {
                    if let Ok(relations) = storage
                        .get_relations(&event.event_id, Some(rel_type), None, 1, None)
                        .await
                        && !relations.is_empty()
                    {
                        ids.insert(event.event_id.clone());
                    }
                }
            }
            Some(ids)
        } else {
            None
        };

    // Include message events and membership events, but exclude other state events
    let chunk: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| !e.is_state() || e.event_type == et::MEMBER)
        .filter(|e| {
            // If rel_type filter is active, only include events with matching relations
            if let Some(ref ids) = related_event_ids {
                ids.contains(&e.event_id)
            } else {
                true
            }
        })
        .map(|e| {
            e.to_client_event()
                .with_membership(Membership::Join.as_str())
                .into_json()
        })
        .collect();

    // If lazy_load_members, include m.room.member state for senders in the chunk
    let state = if lazy_load {
        let mut seen_senders = std::collections::HashSet::new();
        let mut state_events = Vec::new();
        for event in &events {
            if seen_senders.insert(event.sender.clone())
                && let Ok(member_event) = storage
                    .get_state_event(&room_id, et::MEMBER, &event.sender)
                    .await
            {
                state_events.push(member_event.to_client_event().into_json());
            }
        }
        Some(state_events)
    } else {
        None
    };

    Ok(Json(MessagesResponse {
        chunk,
        start,
        end,
        state,
    }))
}

// -- PUT /rooms/{roomId}/state/{eventType}/{stateKey} --

async fn set_state_event(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, event_type, state_key)): Path<(String, String, String)>,
    MatrixJson(content): MatrixJson<serde_json::Value>,
) -> Result<Json<SendEventResponse>, MatrixError> {
    do_set_state(&state, &auth, &room_id, &event_type, &state_key, content).await
}

// -- PUT /rooms/{roomId}/state/{eventType} (empty state_key) --

async fn set_state_event_no_key(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, event_type)): Path<(String, String)>,
    MatrixJson(content): MatrixJson<serde_json::Value>,
) -> Result<Json<SendEventResponse>, MatrixError> {
    do_set_state(&state, &auth, &room_id, &event_type, "", content).await
}

async fn do_set_state(
    state: &AppState,
    auth: &AuthenticatedUser,
    room_id: &str,
    event_type: &str,
    state_key: &str,
    content: serde_json::Value,
) -> Result<Json<SendEventResponse>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user is joined
    let membership = require_membership(storage, &sender, room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Check power levels — user must have sufficient PL to send this state event.
    // Exception: users can always update their own m.room.member event (profile changes).
    let is_own_member_event = event_type == et::MEMBER && state_key == sender;
    // Per spec: when state_key starts with @ and matches sender, the user only
    // needs users_default (typically 0) rather than state_default (typically 50).
    let is_own_state_key = state_key == sender;

    if !is_own_member_event {
        let power_levels = storage
            .get_state_event(room_id, et::POWER_LEVELS, "")
            .await
            .ok();

        if let Some(ref pl_event) = power_levels {
            let user_pl = pl_event
                .content
                .get("users")
                .and_then(|u| u.get(&sender))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            // For state events, required PL comes from events[event_type], or state_default.
            // Exception: if state_key matches sender, use users_default instead of state_default.
            let default_required = if is_own_state_key {
                pl_event
                    .content
                    .get("users_default")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            } else {
                pl_event
                    .content
                    .get("state_default")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(50)
            };

            let required_pl = pl_event
                .content
                .get("events")
                .and_then(|ev| ev.get(event_type))
                .and_then(|v| v.as_i64())
                .unwrap_or(default_required);

            if user_pl < required_pl {
                return Err(MatrixError::forbidden(format!(
                    "Insufficient power level: need {required_pl}, have {user_pl}"
                )));
            }
        }
    }

    // Owned state: if state_key looks like a user ID, only that user can set it.
    // Exception: m.room.member events have their own authorization rules.
    if event_type != "m.room.member" && state_key.starts_with('@') && state_key != sender {
        return Err(MatrixError::forbidden(
            "Cannot set state with another user's ID as state_key",
        ));
    }

    // Validate content is re-serializable (catches NaN, Infinity, etc.)
    let content_str = serde_json::to_string(&content).map_err(|e| {
        MatrixError::bad_json(format!("Event content contains invalid JSON values: {e}"))
    })?;

    if content_str.len() > 65536 {
        return Err(MatrixError::too_large("Event content too large"));
    }

    // Validate m.room.canonical_alias content
    if event_type == et::CANONICAL_ALIAS {
        if let Some(alias) = content.get("alias").and_then(|a| a.as_str())
            && !alias.is_empty()
        {
            // Validate alias format: must start with # and contain :
            if !alias.starts_with('#') || !alias.contains(':') {
                return Err(MatrixError::new(
                    http::StatusCode::BAD_REQUEST,
                    maelstrom_core::matrix::error::ErrorCode::InvalidParam,
                    format!("Invalid alias format: {alias}"),
                ));
            }
            // Alias must exist and point to this room
            match storage.get_room_alias(alias).await {
                Ok(target_room) if target_room != room_id => {
                    return Err(MatrixError::bad_alias("Alias points to a different room"));
                }
                Err(_) => {
                    return Err(MatrixError::bad_alias("Alias does not exist"));
                }
                _ => {}
            }
        }

        // Validate alt_aliases — each must exist and point to this room
        if let Some(alt_aliases) = content.get("alt_aliases").and_then(|a| a.as_array()) {
            for alt in alt_aliases {
                if let Some(alias) = alt.as_str() {
                    // Validate alias format
                    if !alias.starts_with('#') || !alias.contains(':') {
                        return Err(MatrixError::new(
                            http::StatusCode::BAD_REQUEST,
                            maelstrom_core::matrix::error::ErrorCode::InvalidParam,
                            format!("Invalid alias format: {alias}"),
                        ));
                    }
                    match storage.get_room_alias(alias).await {
                        Ok(target_room) if target_room != room_id => {
                            return Err(MatrixError::bad_alias(format!(
                                "Alt alias {alias} points to a different room"
                            )));
                        }
                        Err(_) => {
                            return Err(MatrixError::bad_alias(format!(
                                "Alt alias {alias} does not exist"
                            )));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Idempotency: if current state has identical content, return existing event_id
    if let Ok(existing) = storage
        .get_state_event(room_id, event_type, state_key)
        .await
        && existing.content == content
    {
        return Ok(Json(SendEventResponse {
            event_id: existing.event_id,
        }));
    }

    let event_id = generate_event_id();
    let auth_events =
        crate::handlers::util::select_auth_events(storage, room_id, &sender, event_type).await;
    let event = Pdu {
        event_id: event_id.clone(),
        room_id: room_id.to_string(),
        sender: sender.clone(),
        event_type: event_type.to_string(),
        state_key: Some(state_key.to_string()),
        content,
        origin_server_ts: timestamp_ms(),
        unsigned: None,
        stream_position: 0,
        origin: None,
        auth_events: if auth_events.is_empty() {
            None
        } else {
            Some(auth_events)
        },
        prev_events: None,
        depth: None,
        hashes: None,
        signatures: None,
    };

    storage
        .store_event(&event)
        .await
        .map_err(crate::extractors::storage_error)?;
    storage
        .set_room_state(room_id, event_type, state_key, &event_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // If this is a membership event, update the membership table too
    if event_type == et::MEMBER
        && let Some(ms) = event.content.get("membership").and_then(|v| v.as_str())
    {
        storage
            .set_membership(state_key, room_id, ms)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.to_string(),
        })
        .await;

    Ok(Json(SendEventResponse { event_id }))
}

// -- GET /rooms/{roomId}/state/{eventType}/{stateKey} --

#[derive(Deserialize)]
struct StateEventQuery {
    format: Option<String>,
}

async fn get_state_event(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, event_type, state_key)): Path<(String, String, String)>,
    Query(query): Query<StateEventQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    do_get_state(
        &state,
        &auth,
        &room_id,
        &event_type,
        &state_key,
        query.format.as_deref(),
    )
    .await
}

// -- GET /rooms/{roomId}/state/{eventType} (empty state_key) --

async fn get_state_event_no_key(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, event_type)): Path<(String, String)>,
    Query(query): Query<StateEventQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    do_get_state(
        &state,
        &auth,
        &room_id,
        &event_type,
        "",
        query.format.as_deref(),
    )
    .await
}

async fn do_get_state(
    state: &AppState,
    auth: &AuthenticatedUser,
    room_id: &str,
    event_type: &str,
    state_key: &str,
    format: Option<&str>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user has access
    let membership = require_membership(storage, &sender, room_id).await?;

    // If user has left, return state from when they were in the room
    let event = if membership == Membership::Leave.as_str() {
        if let Ok(member_event) = storage.get_state_event(room_id, et::MEMBER, &sender).await {
            storage
                .get_state_event_at(room_id, event_type, state_key, member_event.stream_position)
                .await
                .map_err(|e| match e {
                    StorageError::NotFound => MatrixError::not_found("State event not found"),
                    other => crate::extractors::storage_error(other),
                })?
        } else {
            storage
                .get_state_event(room_id, event_type, state_key)
                .await
                .map_err(|e| match e {
                    StorageError::NotFound => MatrixError::not_found("State event not found"),
                    other => crate::extractors::storage_error(other),
                })?
        }
    } else {
        storage
            .get_state_event(room_id, event_type, state_key)
            .await
            .map_err(|e| match e {
                StorageError::NotFound => MatrixError::not_found("State event not found"),
                other => crate::extractors::storage_error(other),
            })?
    };

    // ?format=event returns full event, otherwise just content
    if format == Some("event") {
        Ok(Json(event.to_client_event().into_json()))
    } else {
        Ok(Json(event.content))
    }
}

// -- GET /rooms/{roomId}/state --

async fn get_full_state(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    // Check user has access
    let membership = require_membership(storage, &sender, &room_id).await?;

    let events = storage
        .get_current_state(&room_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // If user has left, return state from when they were in the room
    let events = if membership == Membership::Leave.as_str() {
        if let Ok(member_event) = storage.get_state_event(&room_id, et::MEMBER, &sender).await {
            let leave_pos = member_event.stream_position;
            // Keep only state events that existed before the user left
            // For each (event_type, state_key), use the version from before leave
            events
                .into_iter()
                .filter(|e| e.stream_position <= leave_pos)
                .collect()
        } else {
            events
        }
    } else {
        events
    };

    let client_events: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| e.to_client_event().into_json())
        .collect();

    Ok(Json(client_events))
}

// -- PUT /rooms/{roomId}/redact/{eventId}/{txnId} --

/// Request body for `PUT /rooms/{roomId}/redact/{eventId}/{txnId}`.
///
/// An optional `reason` can be provided and will be stored in the redaction
/// event's content.
#[derive(Deserialize)]
struct RedactRequest {
    reason: Option<String>,
}

async fn redact_event(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((room_id, target_event_id, txn_id)): Path<(String, String, String)>,
    MatrixJson(body): MatrixJson<RedactRequest>,
) -> Result<Json<SendEventResponse>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();
    let device_id = auth.device_id.to_string();

    // Check txn_id dedup
    if let Ok(Some(existing_event_id)) = storage.get_txn_event(&device_id, &room_id, &txn_id).await
    {
        return Ok(Json(SendEventResponse {
            event_id: existing_event_id,
        }));
    }

    // Check user is joined
    let membership = require_membership(storage, &sender, &room_id).await?;

    if membership != Membership::Join.as_str() {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Build content
    let mut content = serde_json::Map::new();
    if let Some(reason) = body.reason {
        content.insert("reason".to_string(), serde_json::Value::String(reason));
    }

    // Create the redaction event
    let event_id = generate_event_id();
    let auth_events =
        crate::handlers::util::select_auth_events(storage, &room_id, &sender, et::REDACTION).await;
    let event = Pdu {
        event_id: event_id.clone(),
        room_id: room_id.clone(),
        sender,
        event_type: et::REDACTION.to_string(),
        state_key: None,
        content: serde_json::Value::Object(content),
        origin_server_ts: timestamp_ms(),
        unsigned: Some(serde_json::json!({ "transaction_id": txn_id })),
        stream_position: 0,
        origin: None,
        auth_events: if auth_events.is_empty() {
            None
        } else {
            Some(auth_events)
        },
        prev_events: None,
        depth: None,
        hashes: None,
        signatures: None,
    };

    storage
        .store_event(&event)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Actually redact the target event's content
    let _ = storage.redact_event(&target_event_id).await;

    // Store txn_id mapping
    storage
        .store_txn_id(&device_id, &room_id, &txn_id, &event_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    state
        .notifier()
        .notify(Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(SendEventResponse { event_id }))
}

/// Extract `m.relates_to` from event content and store as a relation record.
async fn extract_and_store_relation(storage: &dyn maelstrom_storage::traits::Storage, event: &Pdu) {
    let relates_to = match event.content.get("m.relates_to") {
        Some(r) => r,
        None => return,
    };

    let rel_type = relates_to
        .get("rel_type")
        .and_then(|r| r.as_str())
        .unwrap_or_default();

    let parent_id = relates_to
        .get("event_id")
        .and_then(|e| e.as_str())
        .unwrap_or_default();

    if rel_type.is_empty() || parent_id.is_empty() {
        return;
    }

    let content_key = if rel_type == "m.annotation" {
        relates_to
            .get("key")
            .and_then(|k| k.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    let relation = maelstrom_storage::traits::RelationRecord {
        event_id: event.event_id.clone(),
        parent_id: parent_id.to_string(),
        room_id: event.room_id.clone(),
        rel_type: rel_type.to_string(),
        sender: event.sender.clone(),
        event_type: event.event_type.clone(),
        content_key,
    };

    let _ = storage.store_relation(&relation).await;
}
