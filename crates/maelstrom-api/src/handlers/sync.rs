use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::error::MatrixError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/_matrix/client/v3/sync", get(sync).post(sliding_sync))
}

// ---------------------------------------------------------------------------
// Traditional GET /sync
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SyncQuery {
    since: Option<String>,
    timeout: Option<u64>,
    full_state: Option<bool>,
    filter: Option<String>,
}

#[derive(Serialize)]
struct SyncResponse {
    next_batch: String,
    rooms: RoomsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_device: Option<SyncToDevice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_lists: Option<DeviceLists>,
}

#[derive(Serialize)]
struct DeviceLists {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    changed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    left: Vec<String>,
}

#[derive(Serialize)]
struct SyncToDevice {
    events: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct RoomsResponse {
    join: HashMap<String, JoinedRoomResponse>,
    invite: HashMap<String, serde_json::Value>,
    leave: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct JoinedRoomResponse {
    timeline: TimelineResponse,
    state: StateResponse,
    ephemeral: EphemeralResponse,
    unread_notifications: UnreadNotifications,
}

#[derive(Serialize)]
struct TimelineResponse {
    events: Vec<serde_json::Value>,
    prev_batch: String,
    limited: bool,
}

#[derive(Serialize)]
struct StateResponse {
    events: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct EphemeralResponse {
    events: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct UnreadNotifications {
    highlight_count: u64,
    notification_count: u64,
}

async fn sync(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<SyncQuery>,
) -> Result<Json<SyncResponse>, MatrixError> {
    let storage = state.storage();
    let user_id = auth.user_id.to_string();

    let since: i64 = query
        .since
        .as_deref()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let _full_state = query.full_state.unwrap_or(false);
    let timeout = query.timeout.unwrap_or(0);
    let device_id = auth.device_id.to_string();

    // Parse sync filter (can be inline JSON or a filter ID)
    let sync_filter: Option<serde_json::Value> = if let Some(ref filter_str) = query.filter {
        if filter_str.starts_with('{') {
            // Inline JSON filter
            serde_json::from_str(filter_str).ok()
        } else {
            // Filter ID — look up from account data
            let filter_key = format!("_maelstrom.filter.{filter_str}");
            storage.get_account_data(&user_id, None, &filter_key).await.ok()
        }
    } else {
        None
    };

    // Extract filter settings
    let include_leave = sync_filter.as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("include_leave"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let timeline_limit = sync_filter.as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("timeline"))
        .and_then(|t| t.get("limit"))
        .and_then(|l| l.as_u64())
        .map(|l| l as usize);

    let timeline_types: Option<Vec<String>> = sync_filter.as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("timeline"))
        .and_then(|t| t.get("types"))
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());

    let state_types: Option<Vec<String>> = sync_filter.as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("state"))
        .and_then(|t| t.get("types"))
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());

    // Get user's rooms by membership state
    let joined_rooms = storage
        .get_joined_rooms(&user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    let invited_rooms = storage
        .get_invited_rooms(&user_id)
        .await
        .unwrap_or_default();

    let left_rooms = storage
        .get_left_rooms(&user_id)
        .await
        .unwrap_or_default();

    // Get current stream position
    let current_position = storage
        .current_stream_position()
        .await
        .map_err(crate::extractors::storage_error)?;

    let is_initial = query.since.is_none();

    // Build the sync response
    let join_map = if is_initial {
        build_initial_sync(storage, &joined_rooms, current_position).await?
    } else {
        build_incremental_sync(storage, &joined_rooms, since, &user_id).await?
    };

    // Fetch to-device messages
    let to_device_events = storage
        .get_to_device_messages(&user_id, &device_id, since)
        .await
        .unwrap_or_default();

    // Delete acknowledged messages (from previous sync batch)
    if since > 0 {
        let _ = storage
            .delete_to_device_messages(&user_id, &device_id, since)
            .await;
    }

    // Check ephemeral events (typing, receipts) before deciding to long-poll
    let join_map = add_ephemeral_events(storage, state.ephemeral(), join_map, &joined_rooms).await?;

    // Check if there are any new events (including ephemeral)
    let has_events = !join_map.is_empty() || !to_device_events.is_empty();

    // If no events and timeout > 0, long-poll (only for incremental sync)
    if !has_events && timeout > 0 && !is_initial {
        let mut rx = state
            .notifier()
            .subscribe(&joined_rooms, Some(&user_id))
            .await;

        tokio::select! {
            _ = rx.recv() => {}
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => {}
        }

        // Re-query after wake-up
        let new_position = storage
            .current_stream_position()
            .await
            .map_err(crate::extractors::storage_error)?;

        let join_map = build_incremental_sync_with_ephemeral(storage, state.ephemeral(), &joined_rooms, since, &user_id).await?;

        let to_device_events = storage
            .get_to_device_messages(&user_id, &device_id, since)
            .await
            .unwrap_or_default();

        // Re-query invited/left rooms after wake-up
        let invited_rooms = storage.get_invited_rooms(&user_id).await.unwrap_or_default();
        let mut invite_map: HashMap<String, serde_json::Value> = HashMap::new();
        for room_id in &invited_rooms {
            let state = storage.get_current_state(room_id).await.unwrap_or_default();
            let invite_state: Vec<serde_json::Value> = state
                .iter()
                .filter(|e| {
                    e.event_type == "m.room.create"
                        || e.event_type == "m.room.join_rules"
                        || e.event_type == "m.room.name"
                        || (e.event_type == "m.room.member" && e.state_key.as_deref() == Some(&user_id))
                })
                .map(|e| e.to_client_event())
                .collect();
            invite_map.insert(room_id.clone(), serde_json::json!({ "invite_state": { "events": invite_state } }));
        }

        // Re-query left rooms after wake-up
        let left_rooms = storage.get_left_rooms(&user_id).await.unwrap_or_default();
        let mut leave_map: HashMap<String, serde_json::Value> = HashMap::new();
        if include_leave || sync_filter.is_none() {
            for room_id in &left_rooms {
                // Get leave position
                let leave_pos = storage.get_state_event(room_id, "m.room.member", &user_id)
                    .await.ok().map(|e| e.stream_position).unwrap_or(new_position);

                // Get events between since and leave_pos
                let mut timeline_events = Vec::new();
                if let Ok(events) = storage.get_room_events(room_id, since, 50, "f").await {
                    for event in events {
                        if event.stream_position <= leave_pos {
                            if let Some(ref types) = timeline_types
                                && !types.contains(&event.event_type) {
                                    continue;
                                }
                            timeline_events.push(event.to_client_event());
                        }
                    }
                }

                leave_map.insert(room_id.clone(), serde_json::json!({
                    "state": { "events": [] },
                    "timeline": { "events": timeline_events, "prev_batch": new_position.to_string(), "limited": false },
                }));
            }
        }

        // Compute device_lists.changed — users in shared rooms with new events
        let device_lists = compute_device_lists(storage, &joined_rooms, &user_id, since).await;

        return Ok(Json(SyncResponse {
            next_batch: new_position.to_string(),
            rooms: RoomsResponse {
                join: join_map,
                invite: invite_map,
                leave: leave_map,
            },
            to_device: Some(SyncToDevice {
                events: to_device_events,
            }),
            device_lists: Some(device_lists),
        }));
    }

    // Ephemeral events already added above (before long-poll check)

    // Build invite section — rooms where user has pending invites
    let mut invite_map: HashMap<String, serde_json::Value> = HashMap::new();
    for room_id in &invited_rooms {
        let state = storage.get_current_state(room_id).await.unwrap_or_default();
        let invite_state: Vec<serde_json::Value> = state
            .iter()
            .filter(|e| {
                e.event_type == "m.room.create"
                    || e.event_type == "m.room.join_rules"
                    || e.event_type == "m.room.name"
                    || (e.event_type == "m.room.member" && e.state_key.as_deref() == Some(&user_id))
            })
            .map(|e| e.to_client_event())
            .collect();

        invite_map.insert(
            room_id.clone(),
            serde_json::json!({ "invite_state": { "events": invite_state } }),
        );
    }

    // Build leave section (only if filter includes leave rooms)
    let mut leave_map: HashMap<String, serde_json::Value> = HashMap::new();
    if include_leave || sync_filter.is_none() {
        for room_id in &left_rooms {
            // Check history visibility
            let history_vis = storage
                .get_state_event(room_id, "m.room.history_visibility", "")
                .await
                .ok()
                .and_then(|e| e.content.get("history_visibility").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| "shared".to_string());

            let can_see_history = history_vis == "world_readable" || history_vis == "shared";

            // Get the user's leave event to determine the departure point
            let leave_pos = storage
                .get_state_event(room_id, "m.room.member", &user_id)
                .await
                .ok()
                .map(|e| e.stream_position)
                .unwrap_or(current_position);

            // Determine effective timeline limit
            let effective_timeline_limit = timeline_limit.unwrap_or(10);

            let mut timeline_events = Vec::new();
            let mut state_events = Vec::new();

            if can_see_history && effective_timeline_limit > 0 {
                if is_initial {
                    // Fetch events backward from the leave position
                    if let Ok(events) = storage.get_room_events(room_id, leave_pos + 1, effective_timeline_limit + 10, "b").await {
                        for event in events {
                            if event.stream_position <= leave_pos {
                                // Apply timeline type filter if specified
                                if let Some(ref types) = timeline_types
                                    && !types.contains(&event.event_type) {
                                        continue;
                                    }
                                timeline_events.push(event.to_client_event());
                                if timeline_events.len() >= effective_timeline_limit {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    // For incremental sync, include the leave membership event
                    if let Ok(member_event) = storage.get_state_event(room_id, "m.room.member", &user_id).await {
                        timeline_events.push(member_event.to_client_event());
                    }
                }
            }

            // Reverse to chronological order (oldest first)
            timeline_events.reverse();

            // If timeline_limit is 0, put relevant events in state section instead
            if effective_timeline_limit == 0 {
                // Include user's leave event and relevant state in state section
                if let Ok(member_event) = storage.get_state_event(room_id, "m.room.member", &user_id).await {
                    state_events.push(member_event.to_client_event());
                }
                // Include state events from before the user left
                if let Ok(events) = storage.get_room_events(room_id, leave_pos + 1, 50, "b").await {
                    for event in events {
                        if event.stream_position <= leave_pos && event.is_state() && event.event_type != "m.room.member" {
                            // Apply state type filter if specified
                            if let Some(ref types) = state_types
                                && !types.contains(&event.event_type) {
                                    continue;
                                }
                            state_events.push(event.to_client_event());
                        }
                    }
                }
            }

            leave_map.insert(
                room_id.clone(),
                serde_json::json!({
                    "state": { "events": state_events },
                    "timeline": { "events": timeline_events, "prev_batch": current_position.to_string(), "limited": false },
                }),
            );
        }
    }

    // Compute device_lists for initial/immediate response
    let device_lists = if !is_initial {
        Some(compute_device_lists(storage, &joined_rooms, &user_id, since).await)
    } else {
        None
    };

    Ok(Json(SyncResponse {
        next_batch: current_position.to_string(),
        rooms: RoomsResponse {
            join: join_map,
            invite: invite_map,
            leave: leave_map,
        },
        to_device: Some(SyncToDevice {
            events: to_device_events,
        }),
        device_lists,
    }))
}

async fn build_initial_sync(
    storage: &dyn maelstrom_storage::traits::Storage,
    joined_rooms: &[String],
    current_position: i64,
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    let mut join_map: HashMap<String, JoinedRoomResponse> = HashMap::new();

    // TODO(M9): Use futures::future::join_all to fetch room data in parallel
    // for better performance when a user is in many rooms.
    for room_id in joined_rooms {
        let state_events = storage
            .get_current_state(room_id)
            .await
            .map_err(crate::extractors::storage_error)?;

        let timeline_events = storage
            .get_room_events(room_id, current_position + 1, 10, "b")
            .await
            .map_err(crate::extractors::storage_error)?;

        let mut timeline_events = timeline_events;
        timeline_events.reverse();

        let prev_batch = timeline_events
            .first()
            .map(|e| e.stream_position.to_string())
            .unwrap_or_else(|| "0".to_string());

        let state_client: Vec<serde_json::Value> =
            state_events.into_iter().map(|e| e.to_client_event()).collect();
        let timeline_client: Vec<serde_json::Value> =
            timeline_events.into_iter().map(|e| e.to_client_event()).collect();

        join_map.insert(
            room_id.clone(),
            JoinedRoomResponse {
                state: StateResponse {
                    events: state_client,
                },
                timeline: TimelineResponse {
                    events: timeline_client,
                    prev_batch,
                    limited: false,
                },
                ephemeral: EphemeralResponse { events: vec![] },
                unread_notifications: UnreadNotifications {
                    highlight_count: 0,
                    notification_count: 0,
                },
            },
        );
    }

    Ok(join_map)
}

async fn build_incremental_sync(
    storage: &dyn maelstrom_storage::traits::Storage,
    joined_rooms: &[String],
    since: i64,
    user_id: &str,
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    let new_events = storage
        .get_events_since(since)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Use a HashSet for O(1) membership checks instead of Vec::contains O(n)
    let joined_set: HashSet<&str> = joined_rooms.iter().map(|s| s.as_str()).collect();

    // Detect newly-joined rooms: rooms where the user has a join event since `since`
    let mut newly_joined: HashSet<String> = HashSet::new();
    for event in &new_events {
        if event.event_type == "m.room.member"
            && event.state_key.as_deref() == Some(user_id)
            && event.content.get("membership").and_then(|m| m.as_str()) == Some("join")
        {
            newly_joined.insert(event.room_id.clone());
        }
    }

    // Separate state events from timeline events per room
    let mut room_state: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let mut room_timeline: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for event in new_events {
        if joined_set.contains(event.room_id.as_str()) {
            let is_newly_joined = newly_joined.contains(&event.room_id);
            let client_event = event.to_client_event();

            if event.is_state() {
                room_state
                    .entry(event.room_id.clone())
                    .or_default()
                    .push(client_event.clone());

                // For existing rooms, state events also go in timeline
                // For newly-joined rooms, state events only go in state section
                if !is_newly_joined {
                    room_timeline
                        .entry(event.room_id.clone())
                        .or_default()
                        .push(client_event);
                }
            } else {
                // Non-state events always go in timeline
                room_timeline
                    .entry(event.room_id.clone())
                    .or_default()
                    .push(client_event);
            }
        }
    }

    let mut join_map: HashMap<String, JoinedRoomResponse> = HashMap::new();
    for room_id in joined_rooms {
        let is_newly_joined = newly_joined.contains(room_id);

        let state_events = if is_newly_joined {
            // For newly joined rooms, include full current state
            let current_state = storage.get_current_state(room_id).await.unwrap_or_default();
            current_state.iter().map(|e| e.to_client_event()).collect()
        } else {
            room_state.remove(room_id).unwrap_or_default()
        };

        let timeline_events = room_timeline.remove(room_id).unwrap_or_default();

        if state_events.is_empty() && timeline_events.is_empty() {
            continue;
        }

        join_map.insert(
            room_id.clone(),
            JoinedRoomResponse {
                state: StateResponse { events: state_events },
                timeline: TimelineResponse {
                    events: timeline_events,
                    prev_batch: since.to_string(),
                    limited: is_newly_joined,
                },
                ephemeral: EphemeralResponse { events: vec![] },
                unread_notifications: UnreadNotifications {
                    highlight_count: 0,
                    notification_count: 0,
                },
            },
        );
    }

    Ok(join_map)
}

async fn build_incremental_sync_with_ephemeral(
    storage: &dyn maelstrom_storage::traits::Storage,
    ephemeral: &maelstrom_core::ephemeral::EphemeralStore,
    joined_rooms: &[String],
    since: i64,
    user_id: &str,
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    let join_map = build_incremental_sync(storage, joined_rooms, since, user_id).await?;
    add_ephemeral_events(storage, ephemeral, join_map, joined_rooms).await
}

/// Compute device_lists.changed — users in shared rooms whose devices may have changed.
async fn compute_device_lists(
    storage: &dyn maelstrom_storage::traits::Storage,
    joined_rooms: &[String],
    my_user_id: &str,
    since: i64,
) -> DeviceLists {
    let mut changed: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Get all users in shared rooms and check for device changes
    let mut room_users: std::collections::HashSet<String> = std::collections::HashSet::new();
    for room_id in joined_rooms {
        if let Ok(members) = storage.get_room_members(room_id, "join").await {
            for member in members {
                if member != my_user_id {
                    room_users.insert(member);
                }
            }
        }
    }

    // Check each user's device change position — only include if changed after `since`
    for user_id in &room_users {
        if let Ok(data) = storage.get_account_data(user_id, None, "_maelstrom.device_change_pos").await
            && let Some(pos) = data.get("pos").and_then(|p| p.as_i64())
                && pos > since {
                    changed.insert(user_id.clone());
                }
    }

    // Also check new member join events since last sync
    if let Ok(new_events) = storage.get_events_since(since).await {
        let joined_set: std::collections::HashSet<&str> = joined_rooms.iter().map(|s| s.as_str()).collect();
        for event in &new_events {
            if event.event_type == "m.room.member"
                && joined_set.contains(event.room_id.as_str())
                && event.sender != my_user_id
                && let Some(membership) = event.content.get("membership").and_then(|m| m.as_str())
                    && membership == "join" {
                        changed.insert(event.sender.clone());
                    }
        }
    }

    DeviceLists {
        changed: changed.into_iter().collect(),
        left: vec![],
    }
}

async fn add_ephemeral_events(
    storage: &dyn maelstrom_storage::traits::Storage,
    ephemeral: &maelstrom_core::ephemeral::EphemeralStore,
    mut join_map: HashMap<String, JoinedRoomResponse>,
    joined_rooms: &[String],
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    for room_id in joined_rooms {
        // Build ephemeral events for this room
        let mut ephemeral_events: Vec<serde_json::Value> = Vec::new();

        // Typing indicators
        let typing_users = ephemeral.get_typing_users(room_id);

        // Always include typing event (even with empty user_ids)
        // so clients can detect when typing has stopped
        ephemeral_events.push(serde_json::json!({
            "type": "m.typing",
            "content": {
                "user_ids": typing_users,
            }
        }));

        // Read receipts
        let receipts = storage
            .get_receipts(room_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(room_id = %room_id, error = %e, "Failed to fetch receipts");
                vec![]
            });

        if !receipts.is_empty() {
            let mut content: HashMap<String, HashMap<String, HashMap<String, serde_json::Value>>> =
                HashMap::new();

            for receipt in &receipts {
                content
                    .entry(receipt.event_id.clone())
                    .or_default()
                    .entry(receipt.receipt_type.clone())
                    .or_default()
                    .insert(
                        receipt.user_id.clone(),
                        serde_json::json!({ "ts": receipt.ts }),
                    );
            }

            ephemeral_events.push(serde_json::json!({
                "type": "m.receipt",
                "content": content,
            }));
        }

        if !ephemeral_events.is_empty() {
            // Get or create room entry
            let room_response = join_map.entry(room_id.clone()).or_insert_with(|| {
                JoinedRoomResponse {
                    state: StateResponse { events: vec![] },
                    timeline: TimelineResponse {
                        events: vec![],
                        prev_batch: "0".to_string(),
                        limited: false,
                    },
                    ephemeral: EphemeralResponse { events: vec![] },
                    unread_notifications: UnreadNotifications {
                        highlight_count: 0,
                        notification_count: 0,
                    },
                }
            });
            room_response.ephemeral.events = ephemeral_events;
        }
    }

    Ok(join_map)
}

// ---------------------------------------------------------------------------
// Sliding Sync — POST /_matrix/client/v3/sync (MSC3575)
// ---------------------------------------------------------------------------

fn default_timeline_limit() -> usize {
    10
}

#[derive(Deserialize)]
struct SlidingSyncRequest {
    #[serde(default)]
    lists: HashMap<String, SlidingSyncList>,
    #[serde(default)]
    room_subscriptions: HashMap<String, RoomSubscription>,
    #[serde(default)]
    extensions: SlidingSyncExtensions,
    pos: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SlidingSyncList {
    #[serde(default)]
    ranges: Vec<[u64; 2]>,
    #[serde(default)]
    required_state: Vec<[String; 2]>,
    #[serde(default = "default_timeline_limit")]
    timeline_limit: usize,
    #[serde(default)]
    sort: Vec<String>,
}

#[derive(Deserialize)]
struct RoomSubscription {
    #[serde(default)]
    required_state: Vec<[String; 2]>,
    #[serde(default = "default_timeline_limit")]
    timeline_limit: usize,
}

#[derive(Deserialize, Default)]
struct SlidingSyncExtensions {
    #[serde(default)]
    to_device: Option<ExtensionConfig>,
    #[serde(default)]
    typing: Option<ExtensionConfig>,
    #[serde(default)]
    receipts: Option<ExtensionConfig>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ExtensionConfig {
    #[serde(default)]
    enabled: bool,
    since: Option<String>,
}

// -- Response types --

#[derive(Serialize)]
struct SlidingSyncResponse {
    pos: String,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    lists: HashMap<String, SlidingSyncListResponse>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    rooms: HashMap<String, SlidingSyncRoomResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extensions: Option<SlidingSyncExtensionsResponse>,
}

#[derive(Serialize)]
struct SlidingSyncListResponse {
    count: u64,
    ops: Vec<SlidingSyncOp>,
}

#[derive(Serialize)]
struct SlidingSyncOp {
    op: String,
    range: [u64; 2],
    room_ids: Vec<String>,
}

#[derive(Serialize)]
struct SlidingSyncRoomResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    required_state: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    timeline: Vec<serde_json::Value>,
    notification_count: u64,
    highlight_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    joined_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invited_count: Option<u64>,
}

#[derive(Serialize)]
struct SlidingSyncExtensionsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    to_device: Option<ToDeviceResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    typing: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipts: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ToDeviceResponse {
    next_batch: String,
    events: Vec<serde_json::Value>,
}

async fn sliding_sync(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<SlidingSyncRequest>,
) -> Result<Json<SlidingSyncResponse>, MatrixError> {
    let storage = state.storage();
    let user_id = auth.user_id.to_string();

    let since: Option<i64> = body.pos.as_deref().and_then(|s| s.parse().ok());
    let is_initial = since.is_none();

    // Get user's joined rooms
    let joined_rooms = storage
        .get_joined_rooms(&user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Get current stream position — this becomes the response `pos`
    let current_position = storage
        .current_stream_position()
        .await
        .map_err(crate::extractors::storage_error)?;

    // Sort rooms by recency: find the latest stream_position in each room.
    // For incremental sync we only need rooms with changes since `pos`.
    let mut room_latest: Vec<(String, i64)> = Vec::with_capacity(joined_rooms.len());
    for room_id in &joined_rooms {
        // Fetch the single most recent event to get its stream_position.
        let events = storage
            .get_room_events(room_id, current_position + 1, 1, "b")
            .await
            .unwrap_or_default();
        let latest = events.first().map(|e| e.stream_position).unwrap_or(0);
        room_latest.push((room_id.clone(), latest));
    }

    // Sort descending by latest activity (most recent first).
    room_latest.sort_by(|a, b| b.1.cmp(&a.1));

    let total_rooms = room_latest.len() as u64;

    // For incremental sync, determine which rooms have new activity.
    let rooms_with_changes: HashSet<String> = if let Some(since_pos) = since {
        room_latest
            .iter()
            .filter(|(_, pos)| *pos > since_pos)
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        // Initial: all rooms are "changed"
        room_latest.iter().map(|(id, _)| id.clone()).collect()
    };

    // Collect the set of room_ids we need to return full room data for.
    let mut rooms_to_fetch: HashMap<String, RoomFetchParams> = HashMap::new();

    // -- Process lists --
    let mut list_responses: HashMap<String, SlidingSyncListResponse> = HashMap::new();

    for (list_name, list) in &body.lists {
        let mut ops = Vec::new();

        for range in &list.ranges {
            let start = range[0] as usize;
            let end = (range[1] as usize).min(room_latest.len().saturating_sub(1));

            let mut range_room_ids = Vec::new();
            for i in start..=end {
                if i < room_latest.len() {
                    let room_id = &room_latest[i].0;
                    // For incremental, only include rooms with changes in the
                    // response room data, but always include room_ids in the
                    // list ops so the client knows the ordering.
                    if rooms_with_changes.contains(room_id) || is_initial {
                        rooms_to_fetch
                            .entry(room_id.clone())
                            .or_insert_with(|| RoomFetchParams {
                                required_state: list.required_state.clone(),
                                timeline_limit: list.timeline_limit,
                            });
                    }
                    range_room_ids.push(room_id.clone());
                }
            }

            ops.push(SlidingSyncOp {
                op: "SYNC".to_string(),
                range: [range[0], end as u64],
                room_ids: range_room_ids,
            });
        }

        list_responses.insert(
            list_name.clone(),
            SlidingSyncListResponse {
                count: total_rooms,
                ops,
            },
        );
    }

    // -- Process room_subscriptions (override list params if also present) --
    for (room_id, sub) in &body.room_subscriptions {
        if joined_rooms.contains(room_id) {
            rooms_to_fetch.insert(
                room_id.clone(),
                RoomFetchParams {
                    required_state: sub.required_state.clone(),
                    timeline_limit: sub.timeline_limit,
                },
            );
        }
    }

    // -- Fetch room data --
    let mut room_responses: HashMap<String, SlidingSyncRoomResponse> = HashMap::new();

    for (room_id, params) in &rooms_to_fetch {
        // Fetch required state
        let required_state = fetch_required_state(
            storage,
            room_id,
            &params.required_state,
            &user_id,
        )
        .await?;

        // Extract room name from state events
        let name = required_state.iter().find_map(|ev| {
            if ev.get("type").and_then(|t| t.as_str()) == Some("m.room.name") {
                ev.get("content")
                    .and_then(|c| c.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        });

        // Fetch timeline events
        let timeline_events = storage
            .get_room_events(room_id, current_position + 1, params.timeline_limit, "b")
            .await
            .map_err(crate::extractors::storage_error)?;

        let mut timeline_events = timeline_events;
        timeline_events.reverse();

        let timeline: Vec<serde_json::Value> = timeline_events
            .into_iter()
            .map(|e| e.to_client_event())
            .collect();

        // Get member counts
        let joined_members = storage
            .get_room_members(room_id, "join")
            .await
            .unwrap_or_default();
        let invited_members = storage
            .get_room_members(room_id, "invite")
            .await
            .unwrap_or_default();

        room_responses.insert(
            room_id.clone(),
            SlidingSyncRoomResponse {
                name,
                required_state,
                timeline,
                notification_count: 0,
                highlight_count: 0,
                initial: if is_initial { Some(true) } else { None },
                joined_count: Some(joined_members.len() as u64),
                invited_count: Some(invited_members.len() as u64),
            },
        );
    }

    // -- Extensions --
    let extensions = build_extensions(storage, state.ephemeral(), &body.extensions, &joined_rooms, current_position, &user_id, auth.device_id.as_ref()).await?;

    Ok(Json(SlidingSyncResponse {
        pos: current_position.to_string(),
        lists: list_responses,
        rooms: room_responses,
        extensions: Some(extensions),
    }))
}

/// Parameters for fetching a room in sliding sync.
struct RoomFetchParams {
    required_state: Vec<[String; 2]>,
    timeline_limit: usize,
}

/// Fetch the required state events for a room based on the filter spec.
///
/// - `["*", "*"]` — all current state
/// - `["m.room.name", ""]` — specific event type with empty state key
/// - `["m.room.member", "$LAZY"]` — only the authenticated user's membership
async fn fetch_required_state(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    required: &[[String; 2]],
    user_id: &str,
) -> Result<Vec<serde_json::Value>, MatrixError> {
    if required.is_empty() {
        return Ok(vec![]);
    }

    // Check for wildcard — return all state.
    let wants_all = required.iter().any(|[t, k]| t == "*" && k == "*");

    if wants_all {
        let all_state = storage
            .get_current_state(room_id)
            .await
            .map_err(crate::extractors::storage_error)?;
        return Ok(all_state.into_iter().map(|e| e.to_client_event()).collect());
    }

    let mut result = Vec::new();
    let mut fetched = HashSet::new();

    for [event_type, state_key] in required {
        let actual_key = if state_key == "$LAZY" {
            user_id.to_string()
        } else {
            state_key.clone()
        };

        let dedup_key = (event_type.clone(), actual_key.clone());
        if fetched.contains(&dedup_key) {
            continue;
        }

        match storage.get_state_event(room_id, event_type, &actual_key).await {
            Ok(event) => {
                result.push(event.to_client_event());
                fetched.insert(dedup_key);
            }
            Err(maelstrom_storage::traits::StorageError::NotFound) => {
                // State event doesn't exist — skip silently.
            }
            Err(e) => return Err(crate::extractors::storage_error(e)),
        }
    }

    Ok(result)
}

/// Build extension responses for typing, receipts, and to_device.
async fn build_extensions(
    storage: &dyn maelstrom_storage::traits::Storage,
    ephemeral: &maelstrom_core::ephemeral::EphemeralStore,
    ext: &SlidingSyncExtensions,
    joined_rooms: &[String],
    current_position: i64,
    user_id: &str,
    device_id: &str,
) -> Result<SlidingSyncExtensionsResponse, MatrixError> {
    // -- to_device --
    let to_device = if ext.to_device.as_ref().is_some_and(|c| c.enabled) {
        let since: i64 = ext
            .to_device
            .as_ref()
            .and_then(|c| c.since.as_deref())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let events = storage
            .get_to_device_messages(user_id, device_id, since)
            .await
            .unwrap_or_default();

        // Delete acknowledged messages
        if since > 0 {
            let _ = storage
                .delete_to_device_messages(user_id, device_id, since)
                .await;
        }

        Some(ToDeviceResponse {
            next_batch: current_position.to_string(),
            events,
        })
    } else {
        None
    };

    // -- typing --
    let typing = if ext.typing.as_ref().is_some_and(|c| c.enabled) {
        let mut rooms: HashMap<String, serde_json::Value> = HashMap::new();
        for room_id in joined_rooms {
            let users = ephemeral.get_typing_users(room_id);
            if !users.is_empty() {
                rooms.insert(
                    room_id.clone(),
                    serde_json::json!({
                        "type": "m.typing",
                        "content": { "user_ids": users }
                    }),
                );
            }
        }
        Some(serde_json::json!({ "rooms": rooms }))
    } else {
        None
    };

    // -- receipts --
    let receipts = if ext.receipts.as_ref().is_some_and(|c| c.enabled) {
        let mut rooms: HashMap<String, serde_json::Value> = HashMap::new();
        for room_id in joined_rooms {
            let room_receipts = storage.get_receipts(room_id).await.unwrap_or_default();
            if !room_receipts.is_empty() {
                let mut content: HashMap<
                    String,
                    HashMap<String, HashMap<String, serde_json::Value>>,
                > = HashMap::new();
                for r in &room_receipts {
                    content
                        .entry(r.event_id.clone())
                        .or_default()
                        .entry(r.receipt_type.clone())
                        .or_default()
                        .insert(r.user_id.clone(), serde_json::json!({ "ts": r.ts }));
                }
                rooms.insert(
                    room_id.clone(),
                    serde_json::json!({
                        "type": "m.receipt",
                        "content": content,
                    }),
                );
            }
        }
        Some(serde_json::json!({ "rooms": rooms }))
    } else {
        None
    };

    Ok(SlidingSyncExtensionsResponse {
        to_device,
        typing,
        receipts,
    })
}
