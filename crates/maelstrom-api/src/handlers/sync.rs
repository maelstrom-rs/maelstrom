//! Sync endpoints -- traditional `/sync` and sliding sync.
//!
//! Implements the following Matrix Client-Server API endpoints
//! ([spec: 10 Sync](https://spec.matrix.org/v1.13/client-server-api/#syncing)):
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | `GET`  | `/_matrix/client/v3/sync` | Traditional sync |
//! | `POST` | `/_matrix/client/v3/sync` | Sliding sync (MSC3575) |
//!
//! # Traditional sync (`GET /sync`)
//!
//! The workhorse of the Matrix protocol. Every client polls this endpoint to
//! receive new events and state changes.
//!
//! ## Initial vs incremental
//!
//! - **Initial sync** (`since` absent): Returns full state and recent timeline
//!   events for every joined room, all pending invites, left rooms (if the
//!   filter includes them), global account data, and to-device messages.
//! - **Incremental sync** (`since` = previous `next_batch`): Returns only the
//!   delta since that stream position -- new events, membership changes, and
//!   newly-joined rooms (which receive full state so the client can render them).
//!
//! ## Long-polling via Notifier
//!
//! When `timeout > 0` and there are no new events, the handler subscribes to
//! the [`Notifier`](crate::notify::Notifier) for the user's joined rooms and
//! waits via `tokio::select!` until either an event arrives or the timeout
//! elapses. After waking, the handler re-queries for changes and returns.
//!
//! ## Response assembly
//!
//! The sync response is organized into sections:
//! - **`rooms.join`** -- Per-room objects containing `state`, `timeline`
//!   (with `prev_batch` for backward pagination), `ephemeral` (typing
//!   indicators, read receipts), `unread_notifications`, `account_data`,
//!   and `summary` (member counts).
//! - **`rooms.invite`** -- Stripped state for rooms with pending invites
//!   (filtered to exclude invites from ignored users).
//! - **`rooms.leave`** -- Timeline and state for rooms the user has departed
//!   (respects `history_visibility`).
//! - **`to_device`** -- Encrypted key-sharing and other device-to-device messages.
//! - **`device_lists`** -- Users whose device lists changed since `since`.
//! - **`account_data`** -- Global account data (push rules, ignored users, etc.).
//! - **`presence`** -- Presence status for users in shared rooms.
//!
//! ## The `since` token system (stream positions)
//!
//! Every event stored in the database is assigned a monotonically increasing
//! `stream_position` (i64). The `next_batch` token in the response is the
//! current maximum stream position, and the client sends it back as `since` on
//! the next request. **Important:** `next_batch` is set to `current_position`
//! (not `current_position + 1`) because the query uses exclusive lower bounds
//! (`stream_position > since`).
//!
//! ## Filters
//!
//! The `filter` query parameter accepts either inline JSON or a stored filter
//! ID (looked up from account data). Supported filter fields include
//! `room.timeline.limit`, `room.timeline.types`, `room.state.types`,
//! `room.state.lazy_load_members`, and `room.include_leave`.
//!
//! # Sliding sync (`POST /sync`, MSC3575)
//!
//! A bandwidth-efficient alternative where the client declares named **room
//! lists** with index ranges (e.g., "show me rooms 0-19 sorted by recency").
//! The server returns data only for rooms within the requested ranges that
//! have changed since the last `pos`.
//!
//! Key differences from traditional sync:
//! - Rooms are sorted by latest activity (most recent first).
//! - The response includes `lists` with `SYNC` operations containing room IDs.
//! - Room data uses `required_state` patterns instead of sending all state.
//! - Extensions (`e2ee`, `to_device`, `account_data`) are opt-in.
//! - The `pos` token works the same way as `next_batch` (stream position).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::room::{HistoryVisibility, Membership, event_type as et};

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::state::AppState;

/// Register sync routes.
///
/// Routes:
/// - `GET  /_matrix/client/v3/sync` -- traditional sync (initial + incremental)
/// - `POST /_matrix/client/v3/sync` -- sliding sync (MSC3575)
pub fn routes() -> Router<AppState> {
    Router::new().route("/_matrix/client/v3/sync", get(sync).post(sliding_sync))
}

// ---------------------------------------------------------------------------
// Traditional GET /sync
// ---------------------------------------------------------------------------

/// Query parameters for `GET /sync`.
///
/// - `since`: stream-position token from a previous `next_batch` (omit for
///   initial sync)
/// - `timeout`: long-poll duration in milliseconds (0 = return immediately)
/// - `full_state`: if true, include full state even for incremental syncs
/// - `filter`: inline JSON filter or a stored filter ID
/// - `set_presence`: override automatic presence (`"online"`, `"offline"`,
///   `"unavailable"`)
#[derive(Deserialize)]
struct SyncQuery {
    since: Option<String>,
    timeout: Option<u64>,
    full_state: Option<bool>,
    filter: Option<String>,
    set_presence: Option<String>,
}

/// Top-level response for `GET /sync`.
///
/// The `next_batch` token must be passed as `since` on the next sync request.
/// All other fields are populated based on what changed since the previous
/// sync (or everything, for an initial sync).
#[derive(Serialize)]
struct SyncResponse {
    next_batch: String,
    rooms: RoomsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_device: Option<SyncToDevice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_lists: Option<DeviceLists>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_data: Option<AccountDataResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence: Option<PresenceResponse>,
}

#[derive(Serialize)]
struct PresenceResponse {
    events: Vec<serde_json::Value>,
}

/// Device list change tracking for E2EE key management.
///
/// `changed` lists user IDs whose device lists have been updated since the
/// previous sync. `left` lists user IDs who no longer share any rooms with
/// the syncing user (their device keys can be discarded).
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

/// Container for per-room data in the sync response.
///
/// - `join`: rooms the user is currently joined to (keyed by room ID)
/// - `invite`: rooms with pending invites (stripped state only)
/// - `leave`: rooms the user has left or been kicked/banned from
#[derive(Serialize)]
struct RoomsResponse {
    join: HashMap<String, JoinedRoomResponse>,
    invite: HashMap<String, serde_json::Value>,
    leave: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct AccountDataResponse {
    events: Vec<serde_json::Value>,
}

/// Per-room data for a joined room in the sync response.
///
/// - `timeline`: recent events with a `prev_batch` token for backward pagination
/// - `state`: state events not already in the timeline (or full state on initial sync)
/// - `ephemeral`: typing notifications, read receipts, and other transient events
/// - `unread_notifications`: highlight and notification counts
/// - `account_data`: per-room account data (tags, etc.)
/// - `summary`: joined/invited member counts
#[derive(Serialize)]
struct JoinedRoomResponse {
    timeline: TimelineResponse,
    state: StateResponse,
    ephemeral: EphemeralResponse,
    unread_notifications: UnreadNotifications,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_data: Option<AccountDataResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<RoomSummary>,
}

#[derive(Serialize)]
struct RoomSummary {
    #[serde(rename = "m.joined_member_count")]
    joined_member_count: u64,
    #[serde(rename = "m.invited_member_count")]
    invited_member_count: u64,
}

/// Timeline section of a joined room.
///
/// `events` contains the actual timeline events in chronological order.
/// `prev_batch` is a stream-position token the client can use to paginate
/// backward via `GET /messages`. `limited` indicates whether the server
/// truncated the timeline (gap detection).
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

    // Handle set_presence query parameter
    if let Some(ref presence) = query.set_presence {
        match presence.as_str() {
            "online" | "offline" | "unavailable" => {
                state.ephemeral().set_presence(&user_id, presence, None);
            }
            _ => {} // ignore invalid values
        }
    } else {
        // Default: set user to "online" on sync
        state.ephemeral().set_presence(&user_id, "online", None);
    }

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
            storage
                .get_account_data(&user_id, None, &filter_key)
                .await
                .ok()
        }
    } else {
        None
    };

    // Extract filter settings
    let include_leave = sync_filter
        .as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("include_leave"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let timeline_limit = sync_filter
        .as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("timeline"))
        .and_then(|t| t.get("limit"))
        .and_then(|l| l.as_u64())
        .map(|l| l as usize);

    let timeline_types: Option<Vec<String>> = sync_filter
        .as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("timeline"))
        .and_then(|t| t.get("types"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

    let state_types: Option<Vec<String>> = sync_filter
        .as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("state"))
        .and_then(|t| t.get("types"))
        .and_then(|t| t.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

    let lazy_load_members = sync_filter
        .as_ref()
        .and_then(|f| f.get("room"))
        .and_then(|r| r.get("state"))
        .and_then(|s| s.get("lazy_load_members"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Get user's rooms by membership state
    let joined_rooms = storage
        .get_joined_rooms(&user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    let invited_rooms = storage
        .get_invited_rooms(&user_id)
        .await
        .unwrap_or_default();

    let left_rooms = storage.get_left_rooms(&user_id).await.unwrap_or_default();

    // Get current stream position
    let current_position = storage
        .current_stream_position()
        .await
        .map_err(crate::extractors::storage_error)?;

    let is_initial = query.since.is_none();

    // Build the sync response
    let join_map = if is_initial {
        build_initial_sync(
            storage,
            &joined_rooms,
            current_position,
            &user_id,
            lazy_load_members,
        )
        .await?
    } else {
        build_incremental_sync(storage, &joined_rooms, since, &user_id, timeline_limit).await?
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

    // Subscribe to notifications BEFORE checking for events. This prevents
    // a race where an event arrives between the check and the long-poll —
    // without this, receipts/typing posted in that window would be missed.
    let mut rx = if timeout > 0 && !is_initial {
        Some(
            state
                .notifier()
                .subscribe(&joined_rooms, Some(&user_id))
                .await,
        )
    } else {
        None
    };

    // Check ephemeral events (typing, receipts) before deciding to long-poll
    let mut join_map =
        add_ephemeral_events(storage, state.ephemeral(), join_map, &joined_rooms).await?;

    // Add per-room account_data to rooms in the join map
    if !is_initial {
        for (room_id, room_response) in join_map.iter_mut() {
            let room_ad = storage
                .get_all_room_account_data(&user_id, room_id)
                .await
                .unwrap_or_default();
            if !room_ad.is_empty() {
                room_response.account_data = Some(AccountDataResponse {
                    events: room_ad
                        .into_iter()
                        .map(|(dtype, content)| {
                            // MSC3391: expose deleted entries with empty content
                            if content.get("_msc3391_deleted").and_then(|v| v.as_bool())
                                == Some(true)
                            {
                                serde_json::json!({ "type": dtype, "content": {} })
                            } else {
                                serde_json::json!({ "type": dtype, "content": content })
                            }
                        })
                        .collect(),
                });
            }
        }
    }

    // Check if there are any new events (including ephemeral)
    let has_events = !join_map.is_empty() || !to_device_events.is_empty();

    // If no events and timeout > 0, long-poll (only for incremental sync)
    if !has_events && let Some(mut rx) = rx.take() {
        tokio::select! {
            _ = rx.recv() => {}
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => {}
        }

        // Re-query after wake-up
        let new_position = storage
            .current_stream_position()
            .await
            .map_err(crate::extractors::storage_error)?;

        let mut join_map = build_incremental_sync_with_ephemeral(
            storage,
            state.ephemeral(),
            &joined_rooms,
            since,
            &user_id,
        )
        .await?;

        // Check per-room account_data for all joined rooms (handles account_data-only changes)
        for room_id in &joined_rooms {
            let room_ad = storage
                .get_all_room_account_data(&user_id, room_id)
                .await
                .unwrap_or_default();
            if !room_ad.is_empty() {
                let ad_response = AccountDataResponse {
                    events: room_ad
                        .into_iter()
                        .map(|(dtype, content)| {
                            if content.get("_msc3391_deleted").and_then(|v| v.as_bool())
                                == Some(true)
                            {
                                serde_json::json!({ "type": dtype, "content": {} })
                            } else {
                                serde_json::json!({ "type": dtype, "content": content })
                            }
                        })
                        .collect(),
                };
                if let Some(room_response) = join_map.get_mut(room_id.as_str()) {
                    room_response.account_data = Some(ad_response);
                } else {
                    join_map.insert(
                        room_id.clone(),
                        JoinedRoomResponse {
                            state: StateResponse { events: vec![] },
                            timeline: TimelineResponse {
                                events: vec![],
                                prev_batch: since.to_string(),
                                limited: false,
                            },
                            ephemeral: EphemeralResponse { events: vec![] },
                            unread_notifications: UnreadNotifications {
                                highlight_count: 0,
                                notification_count: 0,
                            },
                            account_data: Some(ad_response),
                            summary: None,
                        },
                    );
                }
            }
        }

        let to_device_events = storage
            .get_to_device_messages(&user_id, &device_id, since)
            .await
            .unwrap_or_default();

        // Re-query invited/left rooms after wake-up
        let invited_rooms = storage
            .get_invited_rooms(&user_id)
            .await
            .unwrap_or_default();
        let mut invite_map: HashMap<String, serde_json::Value> = HashMap::new();
        for room_id in &invited_rooms {
            let state = storage.get_current_state(room_id).await.unwrap_or_default();
            let invite_state: Vec<serde_json::Value> = state
                .iter()
                .filter(|e| {
                    e.event_type == et::CREATE
                        || e.event_type == et::JOIN_RULES
                        || e.event_type == et::NAME
                        || (e.event_type == et::MEMBER && e.state_key.as_deref() == Some(&user_id))
                })
                .map(|e| e.to_client_event().into_json())
                .collect();
            invite_map.insert(
                room_id.clone(),
                serde_json::json!({ "invite_state": { "events": invite_state } }),
            );
        }

        // Re-query left rooms after wake-up
        let left_rooms = storage.get_left_rooms(&user_id).await.unwrap_or_default();
        let mut leave_map: HashMap<String, serde_json::Value> = HashMap::new();
        if include_leave || sync_filter.is_none() {
            for room_id in &left_rooms {
                // Get leave position
                let leave_pos = storage
                    .get_state_event(room_id, et::MEMBER, &user_id)
                    .await
                    .ok()
                    .map(|e| e.stream_position)
                    .unwrap_or(new_position);

                // Only include rooms where the leave happened after `since`
                if leave_pos <= since {
                    continue;
                }

                // For incremental sync, include the leave membership event
                let mut timeline_events = Vec::new();
                if let Ok(member_event) =
                    storage.get_state_event(room_id, et::MEMBER, &user_id).await
                {
                    timeline_events.push(member_event.to_client_event().into_json());
                }

                leave_map.insert(room_id.clone(), serde_json::json!({
                    "state": { "events": [] },
                    "timeline": { "events": timeline_events, "prev_batch": since.to_string(), "limited": false },
                }));
            }
        }

        // Compute device_lists.changed — users in shared rooms with new events
        let device_lists = compute_device_lists(storage, &joined_rooms, &user_id, since).await;

        // Build global account_data for response
        let global_account_data = build_global_account_data(storage, &user_id, false).await;

        // Build presence events
        let presence =
            build_presence_events(storage, state.ephemeral(), &joined_rooms, &user_id).await;

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
            account_data: global_account_data,
            presence,
        }));
    }

    // Ephemeral events already added above (before long-poll check)

    // Build invite section — rooms where user has pending invites
    // Filter out invites from ignored users
    let ignored_users: std::collections::HashSet<String> = storage
        .get_account_data(&user_id, None, "m.ignored_user_list")
        .await
        .ok()
        .and_then(|v| {
            v.get("ignored_users")
                .and_then(|u| u.as_object())
                .map(|obj| obj.keys().cloned().collect())
        })
        .unwrap_or_default();

    let mut invite_map: HashMap<String, serde_json::Value> = HashMap::new();
    for room_id in &invited_rooms {
        let state = storage.get_current_state(room_id).await.unwrap_or_default();

        // Check if the invite was from an ignored user
        let inviter = state
            .iter()
            .find(|e| {
                e.event_type == et::MEMBER
                    && e.state_key.as_deref() == Some(&user_id)
                    && e.content.get("membership").and_then(|m| m.as_str())
                        == Some(Membership::Invite.as_str())
            })
            .map(|e| e.sender.clone());

        if let Some(ref sender) = inviter
            && ignored_users.contains(sender)
        {
            continue; // Skip invites from ignored users
        }

        let invite_state: Vec<serde_json::Value> = state
            .iter()
            .filter(|e| {
                e.event_type == et::CREATE
                    || e.event_type == et::JOIN_RULES
                    || e.event_type == et::NAME
                    || (e.event_type == et::MEMBER && e.state_key.as_deref() == Some(&user_id))
            })
            .map(|e| e.to_client_event().into_json())
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
            // For incremental sync, only include rooms where the leave happened after `since`
            if !is_initial {
                let leave_pos = storage
                    .get_state_event(room_id, et::MEMBER, &user_id)
                    .await
                    .ok()
                    .map(|e| e.stream_position)
                    .unwrap_or(0);

                if leave_pos <= since {
                    continue; // Left before the since token — skip
                }
            }

            // Check history visibility
            let history_vis = storage
                .get_state_event(room_id, et::HISTORY_VISIBILITY, "")
                .await
                .ok()
                .and_then(|e| {
                    e.content
                        .get("history_visibility")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| HistoryVisibility::Shared.as_str().to_string());

            let can_see_history = HistoryVisibility::parse(&history_vis)
                .map(|h| h.visible_to_departed())
                .unwrap_or(false);

            // Get the user's leave event to determine the departure point
            let leave_pos = storage
                .get_state_event(room_id, et::MEMBER, &user_id)
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
                    if let Ok(events) = storage
                        .get_room_events(room_id, leave_pos + 1, effective_timeline_limit + 10, "b")
                        .await
                    {
                        for event in events {
                            if event.stream_position <= leave_pos {
                                // Apply timeline type filter if specified
                                if let Some(ref types) = timeline_types
                                    && !types.contains(&event.event_type)
                                {
                                    continue;
                                }
                                timeline_events.push(event.to_client_event().into_json());
                                if timeline_events.len() >= effective_timeline_limit {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    // For incremental sync, include the leave membership event
                    if let Ok(member_event) =
                        storage.get_state_event(room_id, et::MEMBER, &user_id).await
                    {
                        timeline_events.push(member_event.to_client_event().into_json());
                    }
                }
            }

            // Reverse to chronological order (oldest first)
            timeline_events.reverse();

            // If timeline_limit is 0, put relevant events in state section instead
            if effective_timeline_limit == 0 {
                // Include user's leave event and relevant state in state section
                if let Ok(member_event) =
                    storage.get_state_event(room_id, et::MEMBER, &user_id).await
                {
                    state_events.push(member_event.to_client_event().into_json());
                }
                // Include state events from before the user left
                if let Ok(events) = storage
                    .get_room_events(room_id, leave_pos + 1, 50, "b")
                    .await
                {
                    for event in events {
                        if event.stream_position <= leave_pos
                            && event.is_state()
                            && event.event_type != et::MEMBER
                        {
                            // Apply state type filter if specified
                            if let Some(ref types) = state_types
                                && !types.contains(&event.event_type)
                            {
                                continue;
                            }
                            state_events.push(event.to_client_event().into_json());
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

    // Build global account_data for response
    let global_account_data = build_global_account_data(storage, &user_id, is_initial).await;

    // Build presence events
    let presence = build_presence_events(storage, state.ephemeral(), &joined_rooms, &user_id).await;

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
        account_data: global_account_data,
        presence,
    }))
}

async fn build_initial_sync(
    storage: &dyn maelstrom_storage::traits::Storage,
    joined_rooms: &[String],
    current_position: i64,
    user_id: &str,
    lazy_load_members: bool,
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

        let timeline_client: Vec<serde_json::Value> = timeline_events
            .iter()
            .map(|e| e.to_client_event().into_json())
            .collect();

        let state_client: Vec<serde_json::Value> = if lazy_load_members {
            // Only include m.room.member for senders that appear in the timeline
            let timeline_senders: HashSet<String> =
                timeline_events.iter().map(|e| e.sender.clone()).collect();
            state_events
                .into_iter()
                .filter(|e| {
                    e.event_type != et::MEMBER
                        || e.state_key
                            .as_ref()
                            .is_some_and(|sk| timeline_senders.contains(sk))
                })
                .map(|e| e.to_client_event().into_json())
                .collect()
        } else {
            state_events
                .into_iter()
                .map(|e| e.to_client_event().into_json())
                .collect()
        };

        // Fetch per-room account data
        let room_account_data = storage
            .get_all_room_account_data(user_id, room_id)
            .await
            .unwrap_or_default();
        let room_ad = if room_account_data.is_empty() {
            None
        } else {
            Some(AccountDataResponse {
                events: room_account_data
                    .into_iter()
                    .map(|(dtype, content)| {
                        if content.get("_msc3391_deleted").and_then(|v| v.as_bool()) == Some(true) {
                            serde_json::json!({ "type": dtype, "content": {} })
                        } else {
                            serde_json::json!({ "type": dtype, "content": content })
                        }
                    })
                    .collect(),
            })
        };

        // Room summary: member counts
        let joined_count = storage
            .get_room_members(room_id, Membership::Join.as_str())
            .await
            .map(|m| m.len() as u64)
            .unwrap_or(0);
        let invited_count = storage
            .get_room_members(room_id, Membership::Invite.as_str())
            .await
            .map(|m| m.len() as u64)
            .unwrap_or(0);

        let state_client = annotate_membership(state_client, Membership::Join.as_str());
        let timeline_client = annotate_membership(timeline_client, Membership::Join.as_str());

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
                account_data: room_ad,
                summary: Some(RoomSummary {
                    joined_member_count: joined_count,
                    invited_member_count: invited_count,
                }),
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
    timeline_limit: Option<usize>,
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    let new_events = storage
        .get_events_since(since)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Use a HashSet for O(1) membership checks instead of Vec::contains O(n)
    let joined_set: HashSet<&str> = joined_rooms.iter().map(|s| s.as_str()).collect();

    // Detect newly-joined rooms: rooms where the user transitioned to "join" since `since`.
    // A m.room.member event with membership=join is only "newly joined" if the user was
    // NOT already joined at the since position (i.e. profile-only updates don't count).
    let mut newly_joined: HashSet<String> = HashSet::new();
    for event in &new_events {
        if event.event_type == et::MEMBER
            && event.state_key.as_deref() == Some(user_id)
            && event.content.get("membership").and_then(|m| m.as_str())
                == Some(Membership::Join.as_str())
        {
            // Check if the user was already joined at the since position
            let was_joined = storage
                .get_state_event_at(&event.room_id, et::MEMBER, user_id, since)
                .await
                .ok()
                .and_then(|e| {
                    e.content
                        .get("membership")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                == Some(Membership::Join.as_str().to_string());

            if !was_joined {
                newly_joined.insert(event.room_id.clone());
            }
        }
    }

    // Separate state events from timeline events per room
    let mut room_state: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let mut room_timeline: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for event in new_events {
        if joined_set.contains(event.room_id.as_str()) {
            let is_newly_joined = newly_joined.contains(&event.room_id);
            let client_event = event.to_client_event().into_json();

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
            current_state
                .iter()
                .map(|e| e.to_client_event().into_json())
                .collect()
        } else {
            room_state.remove(room_id).unwrap_or_default()
        };

        let mut timeline_events = room_timeline.remove(room_id).unwrap_or_default();

        // Apply timeline limit and set limited/prev_batch for gaps
        let effective_limit = timeline_limit.unwrap_or(20);
        let limited = is_newly_joined || timeline_events.len() > effective_limit;
        if timeline_events.len() > effective_limit {
            // Keep only the most recent events (last N)
            timeline_events = timeline_events.split_off(timeline_events.len() - effective_limit);
        }

        if state_events.is_empty() && timeline_events.is_empty() {
            continue;
        }

        // Room summary for incremental sync (include when there are membership changes)
        let has_membership_change = timeline_events
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some(et::MEMBER))
            || state_events
                .iter()
                .any(|e| e.get("type").and_then(|t| t.as_str()) == Some(et::MEMBER));

        let summary = if is_newly_joined || has_membership_change {
            let joined_count = storage
                .get_room_members(room_id, Membership::Join.as_str())
                .await
                .map(|m| m.len() as u64)
                .unwrap_or(0);
            let invited_count = storage
                .get_room_members(room_id, Membership::Invite.as_str())
                .await
                .map(|m| m.len() as u64)
                .unwrap_or(0);
            Some(RoomSummary {
                joined_member_count: joined_count,
                invited_member_count: invited_count,
            })
        } else {
            None
        };

        // prev_batch: when limited, use the stream_position of the first timeline event
        let prev_batch = if limited {
            timeline_events
                .first()
                .and_then(|e| e.get("origin_server_ts")) // Use first event as anchor
                .map(|_| {
                    // Get the stream_position from the first timeline event
                    // We need to use the event's position, stored before to_client_event conversion
                    since.to_string() // Fallback: use since token
                })
                .unwrap_or_else(|| since.to_string())
        } else {
            since.to_string()
        };

        // Add unsigned.membership per spec — for joined rooms it's always "join"
        let state_events = annotate_membership(state_events, Membership::Join.as_str());
        let timeline_events = annotate_membership(timeline_events, Membership::Join.as_str());

        join_map.insert(
            room_id.clone(),
            JoinedRoomResponse {
                state: StateResponse {
                    events: state_events,
                },
                timeline: TimelineResponse {
                    events: timeline_events,
                    prev_batch,
                    limited,
                },
                ephemeral: EphemeralResponse { events: vec![] },
                unread_notifications: UnreadNotifications {
                    highlight_count: 0,
                    notification_count: 0,
                },
                account_data: None,
                summary,
            },
        );
    }

    Ok(join_map)
}

/// Add `unsigned.membership` to a list of client events.
fn annotate_membership(events: Vec<serde_json::Value>, membership: &str) -> Vec<serde_json::Value> {
    events
        .into_iter()
        .map(|mut e| {
            if let Some(obj) = e.as_object_mut() {
                let unsigned = obj
                    .entry("unsigned")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(u) = unsigned.as_object_mut() {
                    u.insert("membership".to_string(), serde_json::json!(membership));
                }
            }
            e
        })
        .collect()
}

async fn build_incremental_sync_with_ephemeral(
    storage: &dyn maelstrom_storage::traits::Storage,
    ephemeral: &maelstrom_core::matrix::ephemeral::EphemeralStore,
    joined_rooms: &[String],
    since: i64,
    user_id: &str,
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    let join_map = build_incremental_sync(storage, joined_rooms, since, user_id, None).await?;
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
        if let Ok(members) = storage
            .get_room_members(room_id, Membership::Join.as_str())
            .await
        {
            for member in members {
                if member != my_user_id {
                    room_users.insert(member);
                }
            }
        }
    }

    // Check each user's device change position — only include if changed after `since`
    for user_id in &room_users {
        if let Ok(data) = storage
            .get_account_data(user_id, None, "_maelstrom.device_change_pos")
            .await
            && let Some(pos) = data.get("pos").and_then(|p| p.as_i64())
            && pos >= since
        {
            changed.insert(user_id.clone());
        }
    }

    // Also check new member join/leave events since last sync.
    // When a user joins a shared room, they should appear in device_lists.changed.
    // When a user leaves all shared rooms, they should appear in device_lists.left.
    let mut left_candidates: std::collections::HashSet<String> = std::collections::HashSet::new();

    if let Ok(new_events) = storage.get_events_since(since).await {
        let joined_set: std::collections::HashSet<&str> =
            joined_rooms.iter().map(|s| s.as_str()).collect();
        for event in &new_events {
            if event.event_type == et::MEMBER && joined_set.contains(event.room_id.as_str()) {
                let target_user = event.state_key.as_deref().unwrap_or(&event.sender);

                if let Some(membership) = event.content.get("membership").and_then(|m| m.as_str()) {
                    // When WE join a room, all existing members should appear in changed
                    // (we need their device keys now that we share a room).
                    if target_user == my_user_id && membership == Membership::Join.as_str() {
                        if let Ok(members) = storage
                            .get_room_members(&event.room_id, Membership::Join.as_str())
                            .await
                        {
                            for member in members {
                                if member != my_user_id {
                                    changed.insert(member);
                                }
                            }
                        }
                        continue;
                    }

                    if target_user == my_user_id {
                        continue;
                    }

                    match membership {
                        m if m == Membership::Join.as_str() => {
                            changed.insert(target_user.to_string());
                        }
                        m if m == Membership::Leave.as_str() || m == Membership::Ban.as_str() => {
                            left_candidates.insert(target_user.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Users who left are only in device_lists.left if they share NO remaining rooms
    let mut left: Vec<String> = Vec::new();
    for user_id in left_candidates {
        if !room_users.contains(&user_id) {
            left.push(user_id);
        }
    }

    DeviceLists {
        changed: changed.into_iter().collect(),
        left,
    }
}

async fn add_ephemeral_events(
    storage: &dyn maelstrom_storage::traits::Storage,
    ephemeral: &maelstrom_core::matrix::ephemeral::EphemeralStore,
    mut join_map: HashMap<String, JoinedRoomResponse>,
    joined_rooms: &[String],
) -> Result<HashMap<String, JoinedRoomResponse>, MatrixError> {
    for room_id in joined_rooms {
        // Build ephemeral events for this room
        let mut ephemeral_events: Vec<serde_json::Value> = Vec::new();

        // Always include typing indicators — even an empty user_ids list is
        // meaningful (it tells the client that typing has stopped).
        let typing_users = ephemeral.get_typing_users(room_id);
        let receipts = storage.get_receipts(room_id).await.unwrap_or_else(|e| {
            tracing::warn!(room_id = %room_id, error = %e, "Failed to fetch receipts");
            vec![]
        });

        ephemeral_events.push(serde_json::json!({
            "type": "m.typing",
            "content": {
                "user_ids": typing_users
            }
        }));

        if !receipts.is_empty() {
            let mut content: HashMap<String, HashMap<String, HashMap<String, serde_json::Value>>> =
                HashMap::new();

            for receipt in &receipts {
                let mut receipt_data = serde_json::json!({ "ts": receipt.ts });
                if !receipt.thread_id.is_empty() {
                    receipt_data["thread_id"] =
                        serde_json::Value::String(receipt.thread_id.clone());
                }
                content
                    .entry(receipt.event_id.clone())
                    .or_default()
                    .entry(receipt.receipt_type.clone())
                    .or_default()
                    .insert(receipt.user_id.clone(), receipt_data);
            }

            ephemeral_events.push(serde_json::json!({
                "type": "m.receipt",
                "content": content,
            }));
        }

        // Determine whether there's something worth reporting beyond empty typing.
        let has_active_ephemeral =
            ephemeral_events
                .iter()
                .any(|e| match e.get("type").and_then(|t| t.as_str()) {
                    Some("m.typing") => e
                        .get("content")
                        .and_then(|c| c.get("user_ids"))
                        .and_then(|u| u.as_array())
                        .is_some_and(|a| !a.is_empty()),
                    _ => true,
                });

        if let Some(room_response) = join_map.get_mut(room_id.as_str()) {
            // Room already in response — always attach ephemeral events.
            room_response.ephemeral.events = ephemeral_events;
        } else if has_active_ephemeral {
            // New room entry only for active typing or receipts.
            join_map.insert(
                room_id.clone(),
                JoinedRoomResponse {
                    state: StateResponse { events: vec![] },
                    timeline: TimelineResponse {
                        events: vec![],
                        prev_batch: "0".to_string(),
                        limited: false,
                    },
                    ephemeral: EphemeralResponse {
                        events: ephemeral_events,
                    },
                    unread_notifications: UnreadNotifications {
                        highlight_count: 0,
                        notification_count: 0,
                    },
                    account_data: None,
                    summary: None,
                },
            );
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

/// Build top-level presence events for users in shared rooms.
async fn build_presence_events(
    storage: &dyn maelstrom_storage::traits::Storage,
    ephemeral: &maelstrom_core::matrix::ephemeral::EphemeralStore,
    joined_rooms: &[String],
    user_id: &str,
) -> Option<PresenceResponse> {
    let mut seen_users = HashSet::new();
    let mut events = Vec::new();

    for room_id in joined_rooms {
        if let Ok(members) = storage
            .get_room_members(room_id, Membership::Join.as_str())
            .await
        {
            for member in members {
                if member == user_id || !seen_users.insert(member.clone()) {
                    continue;
                }
                if let Some(presence) = ephemeral.get_presence(&member) {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let last_active_ago = now_ms.saturating_sub(presence.last_active_ts);

                    let mut content = serde_json::json!({
                        "presence": presence.status,
                        "last_active_ago": last_active_ago,
                    });
                    if let Some(msg) = &presence.status_msg {
                        content["status_msg"] = serde_json::Value::String(msg.clone());
                    }

                    events.push(serde_json::json!({
                        "type": "m.presence",
                        "sender": member,
                        "content": content,
                    }));
                }
            }
        }
    }

    // Also include the user's own presence
    if let Some(presence) = ephemeral.get_presence(user_id) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let last_active_ago = now_ms.saturating_sub(presence.last_active_ts);

        let mut content = serde_json::json!({
            "presence": presence.status,
            "last_active_ago": last_active_ago,
        });
        if let Some(msg) = &presence.status_msg {
            content["status_msg"] = serde_json::Value::String(msg.clone());
        }

        events.push(serde_json::json!({
            "type": "m.presence",
            "sender": user_id,
            "content": content,
        }));
    }

    if events.is_empty() {
        None
    } else {
        Some(PresenceResponse { events })
    }
}

/// Build global account_data for inclusion in sync response.
/// On initial sync, always includes push rules and all user account data.
/// On incremental sync, includes account data if any exists (since we don't
/// track per-item change timestamps yet).
async fn build_global_account_data(
    storage: &dyn maelstrom_storage::traits::Storage,
    user_id: &str,
    is_initial: bool,
) -> Option<AccountDataResponse> {
    let mut events = Vec::new();

    // Build push rules: merge default rules with user customizations
    let user_rules = storage
        .get_account_data(user_id, None, "_maelstrom.push_rules")
        .await
        .ok()
        .unwrap_or_else(|| serde_json::json!({}));

    let has_custom_rules = user_rules
        .as_object()
        .map(|o| !o.is_empty())
        .unwrap_or(false);

    // Include push rules on initial sync or when user has custom rules
    if is_initial || has_custom_rules {
        // Start with defaults, merge user overrides
        let mut global = serde_json::json!({
            "override": [
                {
                    "rule_id": ".m.rule.master",
                    "default": true,
                    "enabled": false,
                    "conditions": [],
                    "actions": ["dont_notify"]
                },
                {
                    "rule_id": ".m.rule.suppress_notices",
                    "default": true,
                    "enabled": true,
                    "conditions": [
                        {"kind": "event_match", "key": "content.msgtype", "pattern": "m.notice"}
                    ],
                    "actions": ["dont_notify"]
                }
            ],
            "content": [
                {
                    "rule_id": ".m.rule.contains_user_name",
                    "default": true,
                    "enabled": true,
                    "conditions": [],
                    "actions": ["notify", {"set_tweak": "sound", "value": "default"}, {"set_tweak": "highlight"}],
                    "pattern": user_id.split(':').next().unwrap_or(user_id).trim_start_matches('@')
                }
            ],
            "underride": [
                {
                    "rule_id": ".m.rule.message",
                    "default": true,
                    "enabled": true,
                    "conditions": [
                        {"kind": "event_match", "key": "type", "pattern": "m.room.message"}
                    ],
                    "actions": ["notify"]
                }
            ],
            "sender": [],
            "room": []
        });

        // Merge user custom rules into defaults
        if let Some(user_obj) = user_rules.as_object()
            && let Some(global_obj) = global.as_object_mut()
        {
            for (kind, rules) in user_obj {
                if let Some(user_arr) = rules.as_array() {
                    let kind_arr = global_obj
                        .entry(kind.clone())
                        .or_insert_with(|| serde_json::json!([]))
                        .as_array_mut();
                    if let Some(arr) = kind_arr {
                        for rule in user_arr {
                            // Add user rules that don't overlap with defaults
                            let rule_id = rule.get("rule_id").and_then(|v| v.as_str());
                            if let Some(rid) = rule_id {
                                // Update default rule or add new one
                                if let Some(existing) = arr.iter_mut().find(|r| {
                                    r.get("rule_id").and_then(|v| v.as_str()) == Some(rid)
                                }) {
                                    *existing = rule.clone();
                                } else {
                                    arr.push(rule.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        events.push(serde_json::json!({
            "type": "m.push_rules",
            "content": { "global": global }
        }));
    }

    // Include all user account data
    if let Ok(all_data) = storage.get_all_account_data(user_id).await {
        for (data_type, content) in all_data {
            let is_deleted =
                content.get("_msc3391_deleted").and_then(|v| v.as_bool()) == Some(true);
            if is_deleted && is_initial {
                // On initial sync, skip deleted entries — the client has never
                // seen them so there is nothing to clear.
                continue;
            }
            // MSC3391: deleted account data is stored with a sentinel;
            // expose it to clients as empty content so they clear the key.
            if is_deleted {
                events.push(serde_json::json!({
                    "type": data_type,
                    "content": {},
                }));
            } else {
                events.push(serde_json::json!({
                    "type": data_type,
                    "content": content,
                }));
            }
        }
    }

    if events.is_empty() {
        None
    } else {
        Some(AccountDataResponse { events })
    }
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
        let required_state =
            fetch_required_state(storage, room_id, &params.required_state, &user_id).await?;

        // Extract room name from state events
        let name = required_state.iter().find_map(|ev| {
            if ev.get("type").and_then(|t| t.as_str()) == Some(et::NAME) {
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
            .map(|e| e.to_client_event().into_json())
            .collect();

        // Get member counts
        let joined_members = storage
            .get_room_members(room_id, Membership::Join.as_str())
            .await
            .unwrap_or_default();
        let invited_members = storage
            .get_room_members(room_id, Membership::Invite.as_str())
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
    let extensions = build_extensions(
        storage,
        state.ephemeral(),
        &body.extensions,
        &joined_rooms,
        current_position,
        &user_id,
        auth.device_id.as_ref(),
    )
    .await?;

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
        return Ok(all_state
            .into_iter()
            .map(|e| e.to_client_event().into_json())
            .collect());
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

        match storage
            .get_state_event(room_id, event_type, &actual_key)
            .await
        {
            Ok(event) => {
                result.push(event.to_client_event().into_json());
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
    ephemeral: &maelstrom_core::matrix::ephemeral::EphemeralStore,
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
