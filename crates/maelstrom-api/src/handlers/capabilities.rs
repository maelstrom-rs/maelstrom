use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;
use maelstrom_core::identifiers::DeviceId;
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
            get(get_account_data).put(put_account_data),
        )
        .route(
            "/_matrix/client/v3/user/{userId}/rooms/{roomId}/account_data/{type}",
            get(get_room_account_data).put(put_room_account_data),
        )
        .route("/_matrix/client/v3/pushrules/", get(get_pushrules))
        .route("/_matrix/client/v3/pushers", get(get_pushers))
        .route("/_matrix/client/v3/pushers/set", post(set_pushers))
        .route(
            "/_matrix/client/v3/pushrules/global/{kind}/{ruleId}",
            axum::routing::put(set_pushrule).get(get_pushrule).delete(delete_pushrule),
        )
        .route(
            "/_matrix/client/v3/pushrules/global/{kind}/{ruleId}/enabled",
            axum::routing::put(set_pushrule_enabled).get(get_pushrule_enabled),
        )
        .route(
            "/_matrix/client/v3/pushrules/global/{kind}/{ruleId}/actions",
            axum::routing::put(set_pushrule_actions).get(get_pushrule_actions),
        )
        .route(
            "/_matrix/client/v3/voip/turnServer",
            get(get_turn_server),
        )
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
    let object_fields = [
        "room", "presence", "account_data", "event_fields",
    ];
    if let Some(obj) = body.as_object() {
        for field in &object_fields {
            if let Some(val) = obj.get(*field) {
                // event_fields is allowed to be an array
                if *field == "event_fields" {
                    if !val.is_array() {
                        return Err(MatrixError::bad_json(
                            format!("Filter field '{field}' must be an array"),
                        ));
                    }
                } else if !val.is_object() {
                    return Err(MatrixError::bad_json(
                        format!("Filter field '{field}' must be an object"),
                    ));
                }
            }
        }

        // Validate room sub-fields if room is present
        if let Some(room) = obj.get("room").and_then(|v| v.as_object()) {
            let room_object_fields = [
                "state", "timeline", "ephemeral", "account_data",
            ];
            for field in &room_object_fields {
                if let Some(val) = room.get(*field)
                    && !val.is_object() {
                        return Err(MatrixError::bad_json(
                            format!("Filter field 'room.{field}' must be an object"),
                        ));
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
        .set_account_data(&sender, None, counter_key, &serde_json::json!({ "next": counter + 1 }))
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

    state
        .storage()
        .set_account_data(&sender, None, &data_type, &content)
        .await
        .map_err(crate::extractors::storage_error)?;

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

    state
        .storage()
        .set_account_data(&sender, Some(&room_id), &data_type, &content)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

// -- Push rules / pushers --

async fn get_pushrules() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "global": {
            "override": [],
            "content": [],
            "room": [],
            "sender": [],
            "underride": []
        }
    }))
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
        .set_account_data(&user_id, None, "_maelstrom.pushers", &serde_json::json!({"items": pushers}))
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}

async fn set_pushrule(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

async fn get_pushrule(
    _auth: AuthenticatedUser,
) -> (http::StatusCode, Json<serde_json::Value>) {
    (http::StatusCode::NOT_FOUND, Json(serde_json::json!({"errcode": "M_NOT_FOUND", "error": "Push rule not found"})))
}

async fn delete_pushrule(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

async fn set_pushrule_enabled(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

async fn get_pushrule_enabled(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "enabled": true }))
}

async fn set_pushrule_actions(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({}))
}

async fn get_pushrule_actions(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "actions": ["notify"] }))
}

async fn get_turn_server(
    _auth: AuthenticatedUser,
) -> Json<serde_json::Value> {
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
            return Err(MatrixError::forbidden("Cannot delete another user's device"));
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

    Ok((http::StatusCode::OK, Json(serde_json::json!({}))))
}

// -- Room members --

#[derive(Deserialize)]
struct MembersQuery {
    membership: Option<String>,
    not_membership: Option<String>,
    #[allow(dead_code)]
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
        storage.get_state_event(&room_id, "m.room.member", &sender)
            .await
            .ok()
            .map(|e| e.stream_position)
    } else {
        None
    };

    let events: Vec<serde_json::Value> = current_state
        .iter()
        .filter(|e| {
            if e.event_type != "m.room.member" {
                return false;
            }
            // Filter out events after the user left
            if let Some(lp) = leave_pos
                && e.stream_position > lp {
                    return false;
                }
            let membership = e.content.get("membership").and_then(|m| m.as_str()).unwrap_or("");
            if let Some(ref filter) = query.membership
                && membership != filter {
                    return false;
                }
            if let Some(ref not_filter) = query.not_membership
                && membership == not_filter {
                    return false;
                }
            true
        })
        .map(|e| e.to_client_event())
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
            .set_receipt(&sender, &room_id, "m.read", event_id)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    // Store m.read.private receipt
    if let Some(event_id) = &body.read_private {
        state
            .storage()
            .set_receipt(&sender, &room_id, "m.read.private", event_id)
            .await
            .map_err(crate::extractors::storage_error)?;
    }

    Ok(Json(serde_json::json!({})))
}
