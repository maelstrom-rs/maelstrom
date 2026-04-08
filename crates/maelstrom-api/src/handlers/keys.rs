use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};

use maelstrom_core::error::MatrixError;

use crate::extractors::{AuthenticatedUser, storage_error};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_matrix/client/v3/keys/upload", post(keys_upload))
        .route("/_matrix/client/v3/keys/query", post(keys_query))
        .route("/_matrix/client/v3/keys/claim", post(keys_claim))
        .route("/_matrix/client/v3/keys/changes", get(keys_changes))
        .route(
            "/_matrix/client/v3/keys/device_signing/upload",
            post(keys_device_signing_upload),
        )
        .route(
            "/_matrix/client/v3/keys/signatures/upload",
            post(keys_signatures_upload),
        )
}

/// POST /_matrix/client/v3/keys/upload
///
/// Upload device keys and/or one-time keys.
async fn keys_upload(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let user_id = auth.user_id.to_string();
    let device_id = auth.device_id.to_string();

    // Store device keys if provided
    if let Some(device_keys) = body.get("device_keys") {
        // Validate: user_id in the key must match the authenticated user
        if let Some(key_user_id) = device_keys.get("user_id").and_then(|v| v.as_str())
            && key_user_id != user_id
        {
            return Err(MatrixError::bad_json(
                "device_keys user_id does not match the authenticated user",
            ));
        }
        // Validate: device_id in the key must match the authenticated device
        if let Some(key_device_id) = device_keys.get("device_id").and_then(|v| v.as_str())
            && key_device_id != device_id
        {
            return Err(MatrixError::bad_json(
                "device_keys device_id does not match the authenticated device",
            ));
        }

        storage
            .set_device_keys(&user_id, &device_id, device_keys)
            .await
            .map_err(storage_error)?;

        // Record device list change for sync tracking
        let change_pos = storage.current_stream_position().await.unwrap_or(0);
        let _ = storage
            .set_account_data(
                &user_id,
                None,
                "_maelstrom.device_change_pos",
                &serde_json::json!({"pos": change_pos}),
            )
            .await;
    }

    // Store one-time keys if provided
    if let Some(otks) = body.get("one_time_keys") {
        storage
            .store_one_time_keys(&user_id, &device_id, otks)
            .await
            .map_err(storage_error)?;
    }

    // Return current OTK counts
    let counts = storage
        .count_one_time_keys(&user_id, &device_id)
        .await
        .map_err(storage_error)?;

    Ok(Json(serde_json::json!({
        "one_time_key_counts": counts
    })))
}

/// POST /_matrix/client/v3/keys/query
///
/// Query device keys and cross-signing keys for users.
async fn keys_query(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();

    // Validate: device_keys values must be arrays (lists of device IDs), not objects
    if let Some(device_keys) = body.get("device_keys")
        && let Some(obj) = device_keys.as_object()
    {
        for (uid, val) in obj {
            if !val.is_array() {
                return Err(MatrixError::bad_json(format!(
                    "device_keys value for '{uid}' must be an array of device IDs"
                )));
            }
        }
    }

    // Extract user IDs from the request
    let user_ids: Vec<String> = body
        .get("device_keys")
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    // Get device keys for all requested users
    let mut device_keys = storage
        .get_device_keys(&user_ids)
        .await
        .map_err(storage_error)?;

    // Ensure every queried user has an entry (even if empty)
    if let Some(obj) = device_keys.as_object_mut() {
        for uid in &user_ids {
            obj.entry(uid.clone()).or_insert(serde_json::json!({}));
        }
    }

    // Get cross-signing keys for each user
    let mut master_keys = serde_json::Map::new();
    let mut self_signing_keys = serde_json::Map::new();
    let mut user_signing_keys = serde_json::Map::new();

    for uid in &user_ids {
        let cross_keys = storage
            .get_cross_signing_keys(uid)
            .await
            .map_err(storage_error)?;

        if let Some(obj) = cross_keys.as_object() {
            if let Some(mk) = obj.get("master_key") {
                master_keys.insert(uid.clone(), mk.clone());
            }
            if let Some(ssk) = obj.get("self_signing_key") {
                self_signing_keys.insert(uid.clone(), ssk.clone());
            }
            if let Some(usk) = obj.get("user_signing_key") {
                user_signing_keys.insert(uid.clone(), usk.clone());
            }
        }
    }

    let mut response = serde_json::json!({
        "device_keys": device_keys,
        "failures": {},
    });

    // Only include cross-signing sections if they have data
    if !master_keys.is_empty() {
        response["master_keys"] = serde_json::Value::Object(master_keys);
    }
    if !self_signing_keys.is_empty() {
        response["self_signing_keys"] = serde_json::Value::Object(self_signing_keys);
    }
    if !user_signing_keys.is_empty() {
        response["user_signing_keys"] = serde_json::Value::Object(user_signing_keys);
    }

    Ok(Json(response))
}

/// POST /_matrix/client/v3/keys/claim
///
/// Claim one-time keys for use in establishing encrypted sessions.
async fn keys_claim(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();

    let claims = body
        .get("one_time_keys")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let claimed = storage
        .claim_one_time_keys(&claims)
        .await
        .map_err(storage_error)?;

    Ok(Json(serde_json::json!({
        "one_time_keys": claimed,
        "failures": {}
    })))
}

/// GET /_matrix/client/v3/keys/changes
///
/// Get users whose device lists have changed (stub for now).
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct KeysChangesQuery {
    from: String,
    to: String,
}

async fn keys_changes(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::extract::Query(query): axum::extract::Query<KeysChangesQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let user_id = auth.user_id.to_string();
    let from: i64 = query.from.parse().unwrap_or(0);

    // Get all users in shared rooms
    let joined_rooms = storage.get_joined_rooms(&user_id).await.unwrap_or_default();
    let mut changed: Vec<String> = Vec::new();

    let mut seen = std::collections::HashSet::new();
    for room_id in &joined_rooms {
        if let Ok(members) = storage.get_room_members(room_id, "join").await {
            for member in members {
                if member != user_id && seen.insert(member.clone()) {
                    // Check if this user has a device change after `from`
                    if let Ok(data) = storage
                        .get_account_data(&member, None, "_maelstrom.device_change_pos")
                        .await
                        && let Some(pos) = data.get("pos").and_then(|p| p.as_i64())
                        && pos > from
                    {
                        changed.push(member);
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "changed": changed,
        "left": [],
    })))
}

/// POST /_matrix/client/v3/keys/device_signing/upload
///
/// Upload cross-signing keys.
async fn keys_device_signing_upload(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let user_id = auth.user_id.to_string();

    storage
        .set_cross_signing_keys(&user_id, &body)
        .await
        .map_err(storage_error)?;

    Ok(Json(serde_json::json!({})))
}

/// POST /_matrix/client/v3/keys/signatures/upload
///
/// Upload key signatures (stub -- accepts and returns success).
async fn keys_signatures_upload(_auth: AuthenticatedUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "failures": {} }))
}
