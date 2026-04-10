//! Server capabilities, account data, filters, devices, push rules, and misc endpoints.
//!
//! This module groups several related subsystems:
//!
//! * **Capabilities** -- advertises what optional features the server supports,
//!   such as which room versions are available and the default room version.
//!   Clients use this to decide which features to enable in their UI.
//! * **Event filters** -- server-side filter definitions that clients create
//!   once and reference by ID in `/sync` requests to limit returned data.
//! * **Account data** -- arbitrary per-user (and per-room) JSON blobs that
//!   clients store on the server (e.g. push rules, ignored users, client
//!   settings). Includes unstable MSC3391 deletion support.
//! * **Push rules and pushers** -- control how and when the user receives
//!   notifications. Push rules are the server-side matching engine; pushers
//!   define external delivery targets (e.g. HTTP push gateways, email).
//! * **Devices** -- list, inspect, rename, and delete the user's login sessions.
//! * **Room members and read markers** -- fetch the member list of a room and
//!   update the user's fully-read marker.
//! * **TURN server** -- provides TURN/STUN credentials for VoIP calls.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET`    | `/_matrix/client/v3/capabilities` | Query server capabilities (room versions, etc.) |
//! | `POST`   | `/_matrix/client/v3/user/{userId}/filter` | Create a new event filter |
//! | `GET`    | `/_matrix/client/v3/user/{userId}/filter/{filterId}` | Retrieve a previously created filter |
//! | `GET/PUT/DELETE` | `/_matrix/client/v3/user/{userId}/account_data/{type}` | Global account data |
//! | `GET/PUT/DELETE` | `/_matrix/client/v3/user/{userId}/rooms/{roomId}/account_data/{type}` | Per-room account data |
//! | `DELETE` | `/_matrix/client/unstable/org.matrix.msc3391/user/{userId}/account_data/{type}` | MSC3391 account data deletion |
//! | `DELETE` | `/_matrix/client/unstable/org.matrix.msc3391/user/{userId}/rooms/{roomId}/account_data/{type}` | MSC3391 per-room account data deletion |
//! | `GET`    | `/_matrix/client/v3/pushrules/` | Get all push rules |
//! | `GET`    | `/_matrix/client/v3/pushers` | Get active pushers |
//! | `POST`   | `/_matrix/client/v3/pushers/set` | Set or delete a pusher |
//! | `PUT/GET/DELETE` | `/_matrix/client/v3/pushrules/global/{kind}/{ruleId}` | Manage an individual push rule |
//! | `PUT/GET` | `/_matrix/client/v3/pushrules/global/{kind}/{ruleId}/enabled` | Enable/disable a push rule |
//! | `PUT/GET` | `/_matrix/client/v3/pushrules/global/{kind}/{ruleId}/actions` | Get/set actions for a push rule |
//! | `GET`    | `/_matrix/client/v3/voip/turnServer` | Get TURN server credentials for VoIP |
//! | `GET`    | `/_matrix/client/v3/devices` | List all devices for the current user |
//! | `GET/PUT/DELETE` | `/_matrix/client/v3/devices/{deviceId}` | Get, rename, or delete a device |
//! | `GET`    | `/_matrix/client/v3/rooms/{roomId}/members` | Get the member list for a room |
//! | `POST`   | `/_matrix/client/v3/rooms/{roomId}/read_markers` | Set read markers (fully-read and read-receipt) |
//!
//! # Matrix spec
//!
//! * [Capabilities negotiation](https://spec.matrix.org/v1.12/client-server-api/#capabilities-negotiation)
//! * [Filtering](https://spec.matrix.org/v1.12/client-server-api/#filtering)
//! * [Client config (account data)](https://spec.matrix.org/v1.12/client-server-api/#client-config)
//! * [Push notifications](https://spec.matrix.org/v1.12/client-server-api/#push-notifications)
//! * [Device management](https://spec.matrix.org/v1.12/client-server-api/#device-management)

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::id::DeviceId;
use maelstrom_core::matrix::room::event_type as et;
use maelstrom_storage::traits::StorageError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/capabilities", get(get_capabilities))
        .route(
            "/_matrix/client/v3/user/{userId}/filter",
            post(create_filter),
        )
        .route(
            "/_matrix/client/v3/user/{userId}/filter/{filterId}",
            get(get_filter),
        )
        .route(
            "/_matrix/client/v3/user/{userId}/account_data/{type}",
            get(get_account_data).put(put_account_data).delete(delete_account_data),
        )
        .route(
            "/_matrix/client/v3/user/{userId}/rooms/{roomId}/account_data/{type}",
            get(get_room_account_data).put(put_room_account_data).delete(delete_room_account_data),
        )
        // MSC3391 unstable endpoints for account data deletion
        .route(
            "/_matrix/client/unstable/org.matrix.msc3391/user/{userId}/account_data/{type}",
            axum::routing::delete(delete_account_data),
        )
        .route(
            "/_matrix/client/unstable/org.matrix.msc3391/user/{userId}/rooms/{roomId}/account_data/{type}",
            axum::routing::delete(delete_room_account_data),
        )
        .route("/_matrix/client/v3/pushrules/", get(get_pushrules))
        .route("/_matrix/client/v3/pushers", get(get_pushers))
        .route("/_matrix/client/v3/pushers/set", post(set_pushers))
        .route(
            "/_matrix/client/v3/pushrules/global/{kind}/{ruleId}",
            axum::routing::put(set_pushrule)
                .get(get_pushrule)
                .delete(delete_pushrule),
        )
        .route(
            "/_matrix/client/v3/pushrules/global/{kind}/{ruleId}/enabled",
            axum::routing::put(set_pushrule_enabled).get(get_pushrule_enabled),
        )
        .route(
            "/_matrix/client/v3/pushrules/global/{kind}/{ruleId}/actions",
            axum::routing::put(set_pushrule_actions).get(get_pushrule_actions),
        )
        .route("/_matrix/client/v3/voip/turnServer", get(get_turn_server))
        .route("/_matrix/client/v3/devices", get(list_devices))
        .route(
            "/_matrix/client/v3/devices/{deviceId}",
            get(get_device).put(update_device).delete(delete_device),
        )
        .route(
            "/_matrix/client/v3/rooms/{roomId}/members",
            get(get_room_members),
        )
        .route(
            "/_matrix/client/v3/rooms/{roomId}/read_markers",
            post(post_read_markers),
        )
}

// -- Capabilities --

async fn get_capabilities(_auth: AuthenticatedUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "capabilities": {
            "m.change_password": { "enabled": true },
            "m.room_versions": {
                "default": "11",
                "available": {
                    "1": "stable",
                    "2": "stable",
                    "3": "stable",
                    "4": "stable",
                    "5": "stable",
                    "6": "stable",
                    "7": "stable",
                    "8": "stable",
                    "9": "stable",
                    "10": "stable",
                    "11": "stable"
                }
            }
        }
    }))
}

// -- Filters --

async fn create_filter(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    // Validate: the body must be an object
    if !body.is_object() {
        return Err(MatrixError::bad_json("Filter must be a JSON object"));
    }

    // Validate known filter fields: if present, they must be objects
    let object_fields = ["room", "presence", "account_data", "event_fields"];
    if let Some(obj) = body.as_object() {
        for field in &object_fields {
            if let Some(val) = obj.get(*field) {
                // event_fields is allowed to be an array
                if *field == "event_fields" {
                    if !val.is_array() {
                        return Err(MatrixError::bad_json(format!(
                            "Filter field '{field}' must be an array"
                        )));
                    }
                } else if !val.is_object() {
                    return Err(MatrixError::bad_json(format!(
                        "Filter field '{field}' must be an object"
                    )));
                }
            }
        }

        // Validate room sub-fields if room is present
        if let Some(room) = obj.get("room").and_then(|v| v.as_object()) {
            let room_object_fields = ["state", "timeline", "ephemeral", "account_data"];
            for field in &room_object_fields {
                if let Some(val) = room.get(*field) {
                    if !val.is_object() {
                        return Err(MatrixError::bad_json(format!(
                            "Filter field 'room.{field}' must be an object"
                        )));
                    }
                    // Validate array fields within each room event filter
                    if let Some(filter_obj) = val.as_object() {
                        let array_fields = [
                            "rooms",
                            "not_rooms",
                            "senders",
                            "not_senders",
                            "types",
                            "not_types",
                        ];
                        for af in &array_fields {
                            if let Some(v) = filter_obj.get(*af) {
                                if !v.is_array() {
                                    return Err(MatrixError::bad_json(format!(
                                        "Filter field 'room.{field}.{af}' must be an array"
                                    )));
                                }
                                // Elements must be strings
                                if let Some(arr) = v.as_array() {
                                    for elem in arr {
                                        if !elem.is_string() {
                                            return Err(MatrixError::bad_json(format!(
                                                "Filter field 'room.{field}.{af}' must contain only strings"
                                            )));
                                        }
                                    }
                                    // Validate format for rooms and senders
                                    if *af == "rooms" || *af == "not_rooms" {
                                        for elem in arr {
                                            if let Some(s) = elem.as_str()
                                                && (!s.starts_with('!') || !s.contains(':'))
                                            {
                                                return Err(MatrixError::bad_json(format!(
                                                    "Invalid room ID in filter: {s}"
                                                )));
                                            }
                                        }
                                    }
                                    if *af == "senders" || *af == "not_senders" {
                                        for elem in arr {
                                            if let Some(s) = elem.as_str()
                                                && (!s.starts_with('@') || !s.contains(':'))
                                            {
                                                return Err(MatrixError::bad_json(format!(
                                                    "Invalid user ID in filter: {s}"
                                                )));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Generate a unique filter ID and store the filter via account data
    let sender = auth.user_id.to_string();
    let storage = state.storage();

    // Use a counter stored in account data for filter IDs
    let counter_key = "_maelstrom.filter_counter";
    let counter = storage
        .get_account_data(&sender, None, counter_key)
        .await
        .ok()
        .and_then(|v| v.get("next").and_then(|n| n.as_u64()))
        .unwrap_or(0);

    let filter_id = counter.to_string();

    // Store the filter
    let filter_key = format!("_maelstrom.filter.{filter_id}");
    let _ = storage
        .set_account_data(&sender, None, &filter_key, &body)
        .await;

    // Increment the counter
    let _ = storage
        .set_account_data(
            &sender,
            None,
            counter_key,
            &serde_json::json!({ "next": counter + 1 }),
        )
        .await;

    Ok(Json(serde_json::json!({ "filter_id": filter_id })))
}

async fn get_filter(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, filter_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();
    let filter_key = format!("_maelstrom.filter.{filter_id}");

    let filter = state
        .storage()
        .get_account_data(&sender, None, &filter_key)
        .await
        .map_err(|_| MatrixError::not_found("Filter not found"))?;

    Ok(Json(filter))
}

// -- Account Data --

async fn get_account_data(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, data_type)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    let content = state
        .storage()
        .get_account_data(&sender, None, &data_type)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Account data not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(content))
}

async fn put_account_data(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, data_type)): Path<(String, String)>,
    MatrixJson(content): MatrixJson<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    // MSC3391: PUT with empty object deletes the account data
    if content.as_object().is_some_and(|o| o.is_empty()) {
        state
            .storage()
            .delete_account_data(&sender, None, &data_type)
            .await
            .map_err(crate::extractors::storage_error)?;
    } else {
        state
            .storage()
            .set_account_data(&sender, None, &data_type, &content)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    state
        .notifier()
        .notify(crate::notify::Notification::AccountData { user_id: sender })
        .await;

    Ok(Json(serde_json::json!({})))
}

async fn get_room_account_data(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, room_id, data_type)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    let content = state
        .storage()
        .get_account_data(&sender, Some(&room_id), &data_type)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Account data not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(content))
}

async fn put_room_account_data(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, room_id, data_type)): Path<(String, String, String)>,
    MatrixJson(content): MatrixJson<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    // MSC3391: PUT with empty object deletes the account data
    if content.as_object().is_some_and(|o| o.is_empty()) {
        state
            .storage()
            .delete_account_data(&sender, Some(&room_id), &data_type)
            .await
            .map_err(crate::extractors::storage_error)?;
    } else {
        state
            .storage()
            .set_account_data(&sender, Some(&room_id), &data_type, &content)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // Notify via room event so sync wakes up for per-room account data
    state
        .notifier()
        .notify(crate::notify::Notification::RoomEvent {
            room_id: room_id.clone(),
        })
        .await;

    Ok(Json(serde_json::json!({})))
}

async fn delete_account_data(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, data_type)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    state
        .storage()
        .delete_account_data(&sender, None, &data_type)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

async fn delete_room_account_data(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((_user_id, room_id, data_type)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    state
        .storage()
        .delete_account_data(&sender, Some(&room_id), &data_type)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

// -- Push rules / pushers --

async fn get_pushrules(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let user_rules = get_user_push_rules(state.storage(), &user_id).await;
    let mut global = default_push_rules();

    // Merge user custom rules (user rules override defaults with same rule_id)
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
                        // Replace matching default rule or append
                        let rule_id = rule.get("rule_id").and_then(|v| v.as_str());
                        if let Some(rid) = rule_id {
                            arr.retain(|r| r.get("rule_id").and_then(|v| v.as_str()) != Some(rid));
                        }
                        arr.push(rule.clone());
                    }
                }
            }
        }
    }

    Json(serde_json::json!({ "global": global }))
}

async fn get_pushers(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    let pushers_data = state
        .storage()
        .get_account_data(&user_id, None, "_maelstrom.pushers")
        .await
        .unwrap_or(serde_json::json!({"items": []}));

    let pusher_list = pushers_data
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(Json(serde_json::json!({ "pushers": pusher_list })))
}

#[derive(Deserialize)]
struct SetPusherRequest {
    pushkey: String,
    kind: String,
    app_id: String,
    app_display_name: Option<String>,
    device_display_name: Option<String>,
    lang: Option<String>,
    data: Option<serde_json::Value>,
}

async fn set_pushers(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    MatrixJson(body): MatrixJson<SetPusherRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();
    let access_token = auth.access_token.clone();

    // Get existing pushers (stored as {"items": [...]})
    let existing = state
        .storage()
        .get_account_data(&user_id, None, "_maelstrom.pushers")
        .await
        .unwrap_or(serde_json::json!({"items": []}));

    let mut pushers: Vec<serde_json::Value> = existing
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    if body.kind.is_empty() {
        // Empty kind = delete pusher with this pushkey
        pushers.retain(|p| p.get("pushkey").and_then(|k| k.as_str()) != Some(&body.pushkey));
    } else {
        // Remove existing pusher with same pushkey, then add new one
        pushers.retain(|p| p.get("pushkey").and_then(|k| k.as_str()) != Some(&body.pushkey));
        pushers.push(serde_json::json!({
            "pushkey": body.pushkey,
            "kind": body.kind,
            "app_id": body.app_id,
            "app_display_name": body.app_display_name,
            "device_display_name": body.device_display_name,
            "lang": body.lang,
            "data": body.data,
            "_access_token": access_token,
        }));
    }

    // Store back
    state
        .storage()
        .set_account_data(
            &user_id,
            None,
            "_maelstrom.pushers",
            &serde_json::json!({"items": pushers}),
        )
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

/// Helper to get user's push rules from account_data, or return default rules.
/// Server-default push rules per the Matrix spec.
fn default_push_rules() -> serde_json::Value {
    serde_json::json!({
        "override": [
            {"rule_id": ".m.rule.master", "default": true, "enabled": false, "conditions": [], "actions": []},
            {"rule_id": ".m.rule.suppress_notices", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "content.msgtype", "pattern": "m.notice"}],
             "actions": ["dont_notify"]},
            {"rule_id": ".m.rule.invite_for_me", "default": true, "enabled": true,
             "conditions": [
                 {"kind": "event_match", "key": "type", "pattern": "m.room.member"},
                 {"kind": "event_match", "key": "content.membership", "pattern": "invite"},
                 {"kind": "event_match", "key": "state_key", "pattern": "[the_user]"}
             ],
             "actions": ["notify", {"set_tweak": "sound", "value": "default"}]},
            {"rule_id": ".m.rule.member_event", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "m.room.member"}],
             "actions": ["dont_notify"]},
            {"rule_id": ".m.rule.is_room_mention", "default": true, "enabled": true,
             "conditions": [
                 {"kind": "event_match", "key": "content.m\\.mentions.room", "pattern": "true"},
                 {"kind": "sender_notification_permission", "key": "room"}
             ],
             "actions": ["notify", {"set_tweak": "highlight"}]},
            {"rule_id": ".m.rule.tombstone", "default": true, "enabled": true,
             "conditions": [
                 {"kind": "event_match", "key": "type", "pattern": "m.room.tombstone"},
                 {"kind": "event_match", "key": "state_key", "pattern": ""}
             ],
             "actions": ["notify", {"set_tweak": "highlight"}]},
            {"rule_id": ".m.rule.reaction", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "m.reaction"}],
             "actions": ["dont_notify"]},
            {"rule_id": ".org.matrix.msc3930.rule.poll_response", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "org.matrix.msc3381.poll.response"}],
             "actions": ["dont_notify"]},
            {"rule_id": ".org.matrix.msc3930.rule.poll_start", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "org.matrix.msc3381.poll.start"}],
             "actions": ["notify"]},
            {"rule_id": ".m.rule.room.server_acl", "default": true, "enabled": true,
             "conditions": [
                 {"kind": "event_match", "key": "type", "pattern": "m.room.server_acl"},
                 {"kind": "event_match", "key": "state_key", "pattern": ""}
             ],
             "actions": []},
        ],
        "content": [
            {"rule_id": ".m.rule.contains_user_name", "default": true, "enabled": true,
             "pattern": "[the_user_localpart]",
             "actions": ["notify", {"set_tweak": "sound", "value": "default"}, {"set_tweak": "highlight"}]}
        ],
        "room": [],
        "sender": [],
        "underride": [
            {"rule_id": ".m.rule.call", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "m.call.invite"}],
             "actions": ["notify", {"set_tweak": "sound", "value": "ring"}]},
            {"rule_id": ".m.rule.room_one_to_one", "default": true, "enabled": true,
             "conditions": [
                 {"kind": "room_member_count", "is": "2"},
                 {"kind": "event_match", "key": "type", "pattern": "m.room.message"}
             ],
             "actions": ["notify", {"set_tweak": "sound", "value": "default"}]},
            {"rule_id": ".m.rule.encrypted_room_one_to_one", "default": true, "enabled": true,
             "conditions": [
                 {"kind": "room_member_count", "is": "2"},
                 {"kind": "event_match", "key": "type", "pattern": "m.room.encrypted"}
             ],
             "actions": ["notify", {"set_tweak": "sound", "value": "default"}]},
            {"rule_id": ".m.rule.message", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "m.room.message"}],
             "actions": ["notify"]},
            {"rule_id": ".m.rule.encrypted", "default": true, "enabled": true,
             "conditions": [{"kind": "event_match", "key": "type", "pattern": "m.room.encrypted"}],
             "actions": ["notify"]},
        ]
    })
}

async fn get_user_push_rules(
    storage: &dyn maelstrom_storage::traits::Storage,
    user_id: &str,
) -> serde_json::Value {
    storage
        .get_account_data(user_id, None, "_maelstrom.push_rules")
        .await
        .unwrap_or_else(|_| serde_json::json!({}))
}

async fn save_user_push_rules(state: &AppState, user_id: &str, rules: &serde_json::Value) {
    let _ = state
        .storage()
        .set_account_data(user_id, None, "_maelstrom.push_rules", rules)
        .await;
    // Notify so sync wakes up with new account_data
    state
        .notifier()
        .notify(crate::notify::Notification::AccountData {
            user_id: user_id.to_string(),
        })
        .await;
}

async fn set_pushrule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
    MatrixJson(body): MatrixJson<serde_json::Value>,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let mut rules = get_user_push_rules(state.storage(), &user_id).await;

    if !rules.is_object() {
        rules = serde_json::json!({});
    }
    let rule_obj = rules.as_object_mut().unwrap();
    let kind_arr = rule_obj
        .entry(&kind)
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut();

    if let Some(arr) = kind_arr {
        // Remove existing rule with same ID
        arr.retain(|r| r.get("rule_id").and_then(|v| v.as_str()) != Some(&rule_id));
        // Add new rule
        let mut new_rule = body;
        new_rule["rule_id"] = serde_json::Value::String(rule_id);
        if new_rule.get("enabled").is_none() {
            new_rule["enabled"] = serde_json::Value::Bool(true);
        }
        arr.push(new_rule);
    }

    save_user_push_rules(&state, &user_id, &rules).await;
    Json(serde_json::json!({}))
}

async fn get_pushrule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    // Check user rules first, then defaults
    let user_rules = get_user_push_rules(state.storage(), &user_id).await;
    if let Some(arr) = user_rules.get(&kind).and_then(|v| v.as_array())
        && let Some(rule) = arr
            .iter()
            .find(|r| r.get("rule_id").and_then(|v| v.as_str()) == Some(&rule_id))
    {
        return Ok(Json(rule.clone()));
    }

    // Check default rules
    let defaults = default_push_rules();
    if let Some(arr) = defaults.get(&kind).and_then(|v| v.as_array())
        && let Some(rule) = arr
            .iter()
            .find(|r| r.get("rule_id").and_then(|v| v.as_str()) == Some(&rule_id))
    {
        return Ok(Json(rule.clone()));
    }

    Err(MatrixError::not_found("Push rule not found"))
}

async fn delete_pushrule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let mut rules = get_user_push_rules(state.storage(), &user_id).await;

    if let Some(arr) = rules.get_mut(&kind).and_then(|v| v.as_array_mut()) {
        arr.retain(|r| r.get("rule_id").and_then(|v| v.as_str()) != Some(&rule_id));
    }

    save_user_push_rules(&state, &user_id, &rules).await;
    Json(serde_json::json!({}))
}

async fn set_pushrule_enabled(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
    MatrixJson(body): MatrixJson<serde_json::Value>,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let enabled = body
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut rules = get_user_push_rules(state.storage(), &user_id).await;

    if let Some(arr) = rules.get_mut(&kind).and_then(|v| v.as_array_mut()) {
        for rule in arr.iter_mut() {
            if rule.get("rule_id").and_then(|v| v.as_str()) == Some(&rule_id) {
                rule["enabled"] = serde_json::Value::Bool(enabled);
            }
        }
    }

    save_user_push_rules(&state, &user_id, &rules).await;
    Json(serde_json::json!({}))
}

async fn get_pushrule_enabled(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let rules = get_user_push_rules(state.storage(), &user_id).await;

    if let Some(arr) = rules.get(&kind).and_then(|v| v.as_array())
        && let Some(rule) = arr
            .iter()
            .find(|r| r.get("rule_id").and_then(|v| v.as_str()) == Some(&rule_id))
    {
        return Json(serde_json::json!({
            "enabled": rule.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
        }));
    }

    Json(serde_json::json!({ "enabled": true }))
}

async fn set_pushrule_actions(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
    MatrixJson(body): MatrixJson<serde_json::Value>,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let actions = body
        .get("actions")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    let mut rules = get_user_push_rules(state.storage(), &user_id).await;

    if let Some(arr) = rules.get_mut(&kind).and_then(|v| v.as_array_mut()) {
        for rule in arr.iter_mut() {
            if rule.get("rule_id").and_then(|v| v.as_str()) == Some(&rule_id) {
                rule["actions"] = actions.clone();
            }
        }
    }

    save_user_push_rules(&state, &user_id, &rules).await;
    Json(serde_json::json!({}))
}

async fn get_pushrule_actions(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path((kind, rule_id)): Path<(String, String)>,
) -> Json<serde_json::Value> {
    let user_id = auth.user_id.to_string();
    let rules = get_user_push_rules(state.storage(), &user_id).await;

    if let Some(arr) = rules.get(&kind).and_then(|v| v.as_array())
        && let Some(rule) = arr
            .iter()
            .find(|r| r.get("rule_id").and_then(|v| v.as_str()) == Some(&rule_id))
    {
        return Json(serde_json::json!({
            "actions": rule.get("actions").cloned().unwrap_or(serde_json::json!(["notify"]))
        }));
    }

    Json(serde_json::json!({ "actions": ["notify"] }))
}

async fn get_turn_server(_auth: AuthenticatedUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "uris": [],
        "username": "",
        "password": "",
        "ttl": 86400
    }))
}

// -- Device management --

async fn list_devices(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let devices = state
        .storage()
        .list_devices(&auth.user_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    let device_list: Vec<serde_json::Value> = devices
        .iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id,
                "display_name": d.display_name,
                "last_seen_ip": null,
                "last_seen_ts": null,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "devices": device_list })))
}

async fn get_device(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(device_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let did = DeviceId::new(&device_id);
    let device = state
        .storage()
        .get_device(&auth.user_id, &did)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Device not found"),
            other => crate::extractors::storage_error(other),
        })?;

    Ok(Json(serde_json::json!({
        "device_id": device.device_id,
        "display_name": device.display_name,
        "last_seen_ip": null,
        "last_seen_ts": null,
    })))
}

#[derive(Deserialize)]
struct UpdateDeviceRequest {
    display_name: Option<String>,
}

async fn update_device(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(device_id): Path<String>,
    MatrixJson(body): MatrixJson<UpdateDeviceRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let did = DeviceId::new(&device_id);

    // Check device exists and belongs to user
    let _device = state
        .storage()
        .get_device(&auth.user_id, &did)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::not_found("Device not found"),
            other => crate::extractors::storage_error(other),
        })?;

    // Update display name if provided
    if let Some(name) = &body.display_name {
        state
            .storage()
            .update_device_display_name(&auth.user_id, &did, Some(name))
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    Ok(Json(serde_json::json!({})))
}

#[derive(Deserialize)]
struct DeleteDeviceRequest {
    auth: Option<DeleteDeviceAuth>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct DeleteDeviceAuth {
    #[serde(rename = "type")]
    auth_type: String,
    session: Option<String>,
    password: Option<String>,
    identifier: Option<AuthIdentifier>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct AuthIdentifier {
    #[serde(rename = "type")]
    id_type: Option<String>,
    user: Option<String>,
}

async fn delete_device(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(device_id): Path<String>,
    body: axum::body::Bytes,
) -> Result<(http::StatusCode, Json<serde_json::Value>), MatrixError> {
    // Parse body manually to handle empty body gracefully
    let parsed: Option<DeleteDeviceRequest> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice(&body).ok()
    };
    let did = DeviceId::new(&device_id);

    // Check device belongs to authenticated user
    match state.storage().get_device(&auth.user_id, &did).await {
        Ok(_) => {} // Device belongs to this user
        Err(StorageError::NotFound) => {
            // Device doesn't belong to this user — forbidden
            return Err(MatrixError::forbidden(
                "Cannot delete another user's device",
            ));
        }
        Err(other) => return Err(crate::extractors::storage_error(other)),
    }

    // Require UIA
    let uia = parsed.and_then(|b| b.auth);
    match &uia {
        Some(a) if a.auth_type == "m.login.password" => {
            // Check that the UIA identifier user matches the authenticated user
            if let Some(ref identifier) = a.identifier
                && let Some(ref uia_user) = identifier.user
            {
                let auth_user_id = auth.user_id.to_string();
                if *uia_user != auth_user_id {
                    return Err(MatrixError::forbidden(
                        "UIA auth user does not match the device owner",
                    ));
                }
            }

            let password = a
                .password
                .as_deref()
                .ok_or_else(|| MatrixError::bad_json("Missing password in auth"))?;

            let user = state
                .storage()
                .get_user(auth.user_id.localpart())
                .await
                .map_err(crate::extractors::storage_error)?;

            let hash = user
                .password_hash
                .as_deref()
                .ok_or_else(|| MatrixError::forbidden("Cannot verify password"))?;

            if crate::handlers::util::verify_password(password.to_string(), hash.to_string())
                .await
                .is_err()
            {
                // Wrong password — return 401 with UIA flows so client can retry
                let session = crate::handlers::util::generate_session_id();
                let response = serde_json::json!({
                    "errcode": "M_FORBIDDEN",
                    "error": "Invalid password",
                    "flows": [
                        { "stages": ["m.login.password"] },
                        { "stages": ["m.login.dummy"] }
                    ],
                    "params": {},
                    "session": session
                });
                return Ok((http::StatusCode::UNAUTHORIZED, Json(response)));
            }
        }
        Some(a) if a.auth_type == "m.login.dummy" => {}
        _ => {
            // Return 401 with UIA flows
            let session = crate::handlers::util::generate_session_id();
            let response = serde_json::json!({
                "flows": [
                    { "stages": ["m.login.password"] },
                    { "stages": ["m.login.dummy"] }
                ],
                "params": {},
                "session": session
            });
            return Ok((http::StatusCode::UNAUTHORIZED, Json(response)));
        }
    }

    state
        .storage()
        .remove_device(&auth.user_id, &did)
        .await
        .map_err(crate::extractors::storage_error)?;

    // Clean up device-specific local notification settings (MSC3890)
    let notif_key = format!("org.matrix.msc3890.local_notification_settings.{did}");
    let _ = state
        .storage()
        .delete_account_data(auth.user_id.as_ref(), None, &notif_key)
        .await;

    Ok((http::StatusCode::OK, Json(serde_json::json!({}))))
}

// -- Room members --

#[derive(Deserialize)]
struct MembersQuery {
    membership: Option<String>,
    not_membership: Option<String>,
    at: Option<String>,
}

async fn get_room_members(
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    Query(query): Query<MembersQuery>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let sender = auth.user_id.to_string();

    let current_state = storage
        .get_current_state(&room_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    // If user has left, only return members from before they left
    let membership = storage.get_membership(&sender, &room_id).await.ok();
    let leave_pos = if membership.as_deref() == Some("leave") {
        storage
            .get_state_event(&room_id, et::MEMBER, &sender)
            .await
            .ok()
            .map(|e| e.stream_position)
    } else {
        None
    };

    // If `at` is specified, use it as the position cutoff
    let at_position: Option<i64> = query.at.as_deref().and_then(|s| s.parse().ok());
    let position_limit = at_position.or(leave_pos);

    let events: Vec<serde_json::Value> = current_state
        .iter()
        .filter(|e| {
            if e.event_type != "m.room.member" {
                return false;
            }
            // Filter out events after the position limit (at parameter or leave position)
            if let Some(limit) = position_limit
                && e.stream_position > limit
            {
                return false;
            }
            let membership = e
                .content
                .get("membership")
                .and_then(|m| m.as_str())
                .unwrap_or("");
            if let Some(ref filter) = query.membership
                && membership != filter
            {
                return false;
            }
            if let Some(ref not_filter) = query.not_membership
                && membership == not_filter
            {
                return false;
            }
            true
        })
        .map(|e| e.to_client_event().into_json())
        .collect();

    Ok(Json(serde_json::json!({ "chunk": events })))
}

// -- Read markers --

#[derive(Deserialize)]
#[allow(dead_code)]
struct ReadMarkersRequest {
    #[serde(rename = "m.fully_read")]
    fully_read: Option<String>,
    #[serde(rename = "m.read")]
    read: Option<String>,
    #[serde(rename = "m.read.private")]
    read_private: Option<String>,
}

async fn post_read_markers(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    MatrixJson(body): MatrixJson<ReadMarkersRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let sender = auth.user_id.to_string();

    // Store fully_read as account data
    if let Some(event_id) = &body.fully_read {
        let content = serde_json::json!({ "event_id": event_id });
        state
            .storage()
            .set_account_data(&sender, Some(&room_id), "m.fully_read", &content)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // Store m.read receipt
    if let Some(event_id) = &body.read {
        state
            .storage()
            .set_receipt(&sender, &room_id, "m.read", event_id, None)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // Store m.read.private receipt
    if let Some(event_id) = &body.read_private {
        state
            .storage()
            .set_receipt(&sender, &room_id, "m.read.private", event_id, None)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // Notify sync so the account_data / receipt appears in next sync
    state
        .notifier()
        .notify(crate::notify::Notification::AccountData { user_id: sender })
        .await;

    Ok(Json(serde_json::json!({})))
}
