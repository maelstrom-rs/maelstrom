use axum::extract::{Path, Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::error::MatrixError;
use maelstrom_core::events::pdu::{StoredEvent, generate_event_id, timestamp_ms};
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::notify::Notification;
use crate::state::AppState;

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
    if let Ok(Some(existing_event_id)) = storage.get_txn_event(&device_id, &txn_id).await {
        return Ok(Json(SendEventResponse {
            event_id: existing_event_id,
        }));
    }

    // Check user is joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
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
    let event = StoredEvent {
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
        auth_events: None,
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
        .store_txn_id(&device_id, &txn_id, &event_id)
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
        .get_state_event(&room_id, "m.room.history_visibility", "")
        .await
        .ok()
        .and_then(|ev| {
            ev.content
                .get("history_visibility")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "shared".to_string());

    // Check user membership
    let membership = storage.get_membership(&sender, &room_id).await;

    let is_member = membership.as_deref().map(|m| m == "join").unwrap_or(false);

    // If not a member, check if world_readable
    if !is_member && history_visibility != "world_readable" {
        return Err(MatrixError::forbidden(
            "You are not allowed to view this event",
        ));
    }
    // world_readable: allow access without membership

    let event = storage.get_event(&event_id).await.map_err(|e| match e {
        StorageError::NotFound => MatrixError::not_found("Event not found"),
        other => crate::extractors::storage_error(other),
    })?;

    if event.room_id != room_id {
        return Err(MatrixError::not_found("Event not found"));
    }

    Ok(Json(event.to_client_event()))
}

// -- GET /rooms/{roomId}/messages --

#[derive(Deserialize)]
#[allow(dead_code)]
struct MessagesQuery {
    from: Option<String>,
    to: Option<String>,
    dir: Option<String>,
    limit: Option<usize>,
}

#[derive(Serialize)]
struct MessagesResponse {
    chunk: Vec<serde_json::Value>,
    start: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<String>,
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
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

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

    // For departed users: limit messages to events up to when they left
    let leave_pos = if membership == "leave" {
        storage
            .get_state_event(&room_id, "m.room.member", &sender)
            .await
            .ok()
            .map(|e| e.stream_position)
    } else {
        None
    };

    let events = storage
        .get_room_events(&room_id, from, limit, dir)
        .await
        .map_err(crate::extractors::storage_error)?;

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

    // Include message events and membership events, but exclude other state events
    let chunk: Vec<serde_json::Value> = events
        .into_iter()
        .filter(|e| !e.is_state() || e.event_type == "m.room.member")
        .map(|e| e.to_client_event())
        .collect();

    Ok(Json(MessagesResponse { chunk, start, end }))
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
    let membership = storage
        .get_membership(&sender, room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Check power levels — user must have sufficient PL to send this state event
    let power_levels = storage
        .get_state_event(room_id, "m.room.power_levels", "")
        .await
        .ok();

    if let Some(ref pl_event) = power_levels {
        let user_pl = pl_event
            .content
            .get("users")
            .and_then(|u| u.get(&sender))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        // For state events, required PL comes from events[event_type], or state_default
        let required_pl = pl_event
            .content
            .get("events")
            .and_then(|ev| ev.get(event_type))
            .and_then(|v| v.as_i64())
            .or_else(|| {
                pl_event
                    .content
                    .get("state_default")
                    .and_then(|v| v.as_i64())
            })
            .unwrap_or(50);

        if user_pl < required_pl {
            return Err(MatrixError::forbidden(format!(
                "Insufficient power level: need {required_pl}, have {user_pl}"
            )));
        }
    }

    // Validate content is re-serializable (catches NaN, Infinity, etc.)
    let content_str = serde_json::to_string(&content).map_err(|e| {
        MatrixError::bad_json(format!("Event content contains invalid JSON values: {e}"))
    })?;

    if content_str.len() > 65536 {
        return Err(MatrixError::too_large("Event content too large"));
    }

    // Validate m.room.canonical_alias content
    if event_type == "m.room.canonical_alias" {
        if let Some(alias) = content.get("alias").and_then(|a| a.as_str())
            && !alias.is_empty()
        {
            // Validate alias format: must start with # and contain :
            if !alias.starts_with('#') || !alias.contains(':') {
                return Err(MatrixError::new(
                    http::StatusCode::BAD_REQUEST,
                    maelstrom_core::error::ErrorCode::InvalidParam,
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
                            maelstrom_core::error::ErrorCode::InvalidParam,
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
    let event = StoredEvent {
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
        auth_events: None,
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
    if event_type == "m.room.member"
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
    let membership = storage
        .get_membership(&sender, room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    // If user has left, return state from when they were in the room
    let event = if membership == "leave" {
        if let Ok(member_event) = storage
            .get_state_event(room_id, "m.room.member", &sender)
            .await
        {
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
        Ok(Json(event.to_client_event()))
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
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    let events = storage
        .get_current_state(&room_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // If user has left, return state from when they were in the room
    let events = if membership == "leave" {
        if let Ok(member_event) = storage
            .get_state_event(&room_id, "m.room.member", &sender)
            .await
        {
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

    let client_events: Vec<serde_json::Value> =
        events.into_iter().map(|e| e.to_client_event()).collect();

    Ok(Json(client_events))
}

// -- PUT /rooms/{roomId}/redact/{eventId}/{txnId} --

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
    if let Ok(Some(existing_event_id)) = storage.get_txn_event(&device_id, &txn_id).await {
        return Ok(Json(SendEventResponse {
            event_id: existing_event_id,
        }));
    }

    // Check user is joined
    let membership = storage
        .get_membership(&sender, &room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })?;

    if membership != "join" {
        return Err(MatrixError::forbidden("You are not in this room"));
    }

    // Build content
    let mut content = serde_json::Map::new();
    if let Some(reason) = body.reason {
        content.insert("reason".to_string(), serde_json::Value::String(reason));
    }

    // Create the redaction event
    let event_id = generate_event_id();
    let event = StoredEvent {
        event_id: event_id.clone(),
        room_id: room_id.clone(),
        sender,
        event_type: "m.room.redaction".to_string(),
        state_key: None,
        content: serde_json::Value::Object(content),
        origin_server_ts: timestamp_ms(),
        unsigned: Some(serde_json::json!({ "transaction_id": txn_id })),
        stream_position: 0,
        origin: None,
        auth_events: None,
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
        .store_txn_id(&device_id, &txn_id, &event_id)
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
async fn extract_and_store_relation(
    storage: &dyn maelstrom_storage::traits::Storage,
    event: &StoredEvent,
) {
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
