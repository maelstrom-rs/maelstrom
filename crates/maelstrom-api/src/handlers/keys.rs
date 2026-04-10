//! End-to-end encryption (E2EE) key management.
//!
//! This module implements the Matrix key management endpoints that underpin
//! Megolm-based end-to-end encryption. Three categories of keys flow through
//! these endpoints:
//!
//! * **Device keys** -- the long-lived Curve25519/Ed25519 identity keys for
//!   each device. Uploaded once and queried by other users to establish Olm
//!   sessions.
//! * **One-time keys** -- ephemeral Curve25519 keys consumed during Olm session
//!   setup. The server hands one out each time another device wants to start an
//!   encrypted conversation.
//! * **Cross-signing keys** -- master, self-signing, and user-signing keys that
//!   let a user verify their own devices and other users without per-device
//!   trust.
//!
//! Additionally, server-side **key backup** endpoints allow clients to store
//! encrypted Megolm session keys so they can be recovered on new devices.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST` | `/_matrix/client/v3/keys/upload` | Upload device keys and/or one-time keys for the current device |
//! | `POST` | `/_matrix/client/v3/keys/query` | Download device keys for a set of users |
//! | `POST` | `/_matrix/client/v3/keys/claim` | Claim one-time keys for establishing Olm sessions |
//! | `GET`  | `/_matrix/client/v3/keys/changes` | Get the list of users whose devices have changed since a given point |
//! | `POST` | `/_matrix/client/v3/keys/device_signing/upload` | Upload cross-signing keys (master, self-signing, user-signing) |
//! | `POST` | `/_matrix/client/v3/keys/signatures/upload` | Upload cross-signing signatures for devices or other keys |
//! | `POST` | `/_matrix/client/v3/room_keys/version` | Create a new key backup version |
//! | `GET`  | `/_matrix/client/v3/room_keys/version` | Get the current key backup version |
//! | `GET`  | `/_matrix/client/v3/room_keys/version/{version}` | Get info about a specific backup version |
//! | `PUT`  | `/_matrix/client/v3/room_keys/keys/{roomId}/{sessionId}` | Store a key for a specific session in a room |
//! | `GET`  | `/_matrix/client/v3/room_keys/keys/{roomId}/{sessionId}` | Retrieve a key for a specific session |
//! | `PUT`  | `/_matrix/client/v3/room_keys/keys/{roomId}` | Store keys for all sessions in a room |
//! | `GET`  | `/_matrix/client/v3/room_keys/keys/{roomId}` | Retrieve keys for all sessions in a room |
//! | `PUT`  | `/_matrix/client/v3/room_keys/keys` | Store keys for all rooms |
//! | `GET`  | `/_matrix/client/v3/room_keys/keys` | Retrieve keys for all rooms |
//!
//! # Matrix spec
//!
//! * [End-to-End Encryption](https://spec.matrix.org/v1.12/client-server-api/#end-to-end-encryption)
//! * [Server-side key backups](https://spec.matrix.org/v1.12/client-server-api/#server-side-key-backups)

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};

use maelstrom_core::matrix::error::MatrixError;

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
        .route(
            "/_matrix/client/v3/room_keys/version",
            post(create_key_backup).get(get_key_backup),
        )
        .route(
            "/_matrix/client/v3/room_keys/version/{version}",
            get(get_key_backup_version),
        )
        .route(
            "/_matrix/client/v3/room_keys/keys/{roomId}/{sessionId}",
            axum::routing::put(put_room_key).get(get_room_key),
        )
        .route(
            "/_matrix/client/v3/room_keys/keys/{roomId}",
            axum::routing::put(put_room_keys).get(get_room_keys),
        )
        .route(
            "/_matrix/client/v3/room_keys/keys",
            axum::routing::put(put_all_room_keys).get(get_all_room_keys),
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

        // Validate required fields per spec
        if device_keys.get("algorithms").is_none() || device_keys.get("keys").is_none() {
            return Err(MatrixError::bad_json(
                "device_keys must include 'algorithms' and 'keys' fields",
            ));
        }

        // Validate algorithms is an array
        if !device_keys
            .get("algorithms")
            .map(|a| a.is_array())
            .unwrap_or(false)
        {
            return Err(MatrixError::bad_json(
                "device_keys 'algorithms' must be an array",
            ));
        }

        // Validate keys is an object
        if !device_keys
            .get("keys")
            .map(|k| k.is_object())
            .unwrap_or(false)
        {
            return Err(MatrixError::bad_json(
                "device_keys 'keys' must be an object",
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
/// Upload cross-signing keys. Requires UIA (MSC3967).
async fn keys_device_signing_upload(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<(http::StatusCode, Json<serde_json::Value>), MatrixError> {
    // Require UIA
    let auth_ok = body
        .get("auth")
        .and_then(|a| a.get("type"))
        .and_then(|t| t.as_str());
    match auth_ok {
        Some("m.login.password") => {
            let password = body
                .get("auth")
                .and_then(|a| a.get("password"))
                .and_then(|p| p.as_str())
                .ok_or_else(|| MatrixError::bad_json("Missing password in auth"))?;

            let user = state
                .storage()
                .get_user(auth.user_id.localpart())
                .await
                .map_err(storage_error)?;

            let hash = user
                .password_hash
                .as_deref()
                .ok_or_else(|| MatrixError::forbidden("Cannot verify password"))?;

            if crate::handlers::util::verify_password(password.to_string(), hash.to_string())
                .await
                .is_err()
            {
                let session = crate::handlers::util::generate_session_id();
                return Ok((
                    http::StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "flows": [{"stages": ["m.login.password"]}, {"stages": ["m.login.dummy"]}],
                        "session": session,
                        "errcode": "M_FORBIDDEN",
                        "error": "Invalid password"
                    })),
                ));
            }
        }
        Some("m.login.dummy") => {}
        _ => {
            let session = crate::handlers::util::generate_session_id();
            return Ok((
                http::StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "flows": [{"stages": ["m.login.password"]}, {"stages": ["m.login.dummy"]}],
                    "session": session
                })),
            ));
        }
    }

    let storage = state.storage();
    let user_id = auth.user_id.to_string();

    storage
        .set_cross_signing_keys(&user_id, &body)
        .await
        .map_err(storage_error)?;

    Ok((http::StatusCode::OK, Json(serde_json::json!({}))))
}

/// POST /_matrix/client/v3/keys/signatures/upload
///
/// Upload key signatures (stub -- accepts and returns success).
async fn keys_signatures_upload(_auth: AuthenticatedUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "failures": {} }))
}

/// POST /_matrix/client/v3/room_keys/version — create a new key backup version
async fn create_key_backup(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    // Get current version counter
    let current = state
        .storage()
        .get_account_data(&user_id, None, "_maelstrom.key_backup_version")
        .await
        .ok()
        .and_then(|v| v.get("version").and_then(|n| n.as_u64()))
        .unwrap_or(0);

    let new_version = current + 1;
    let version_str = new_version.to_string();

    // Store the backup info
    let _ = state
        .storage()
        .set_account_data(
            &user_id,
            None,
            &format!("_maelstrom.key_backup.{version_str}"),
            &body,
        )
        .await;

    // Update version counter
    let _ = state
        .storage()
        .set_account_data(
            &user_id,
            None,
            "_maelstrom.key_backup_version",
            &serde_json::json!({"version": new_version}),
        )
        .await;

    Ok(Json(serde_json::json!({ "version": version_str })))
}

/// GET /_matrix/client/v3/room_keys/version — get current key backup version
async fn get_key_backup(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    let current = state
        .storage()
        .get_account_data(&user_id, None, "_maelstrom.key_backup_version")
        .await
        .ok()
        .and_then(|v| v.get("version").and_then(|n| n.as_u64()));

    match current {
        Some(version) => {
            let version_str = version.to_string();
            let info = state
                .storage()
                .get_account_data(
                    &user_id,
                    None,
                    &format!("_maelstrom.key_backup.{version_str}"),
                )
                .await
                .unwrap_or(serde_json::json!({}));

            let mut response = info;
            response["version"] = serde_json::Value::String(version_str);
            response["count"] = serde_json::json!(0);
            response["etag"] = serde_json::Value::String("0".to_string());
            Ok(Json(response))
        }
        None => Err(MatrixError::not_found("No key backup found")),
    }
}

/// GET /_matrix/client/v3/room_keys/version/{version}
async fn get_key_backup_version(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::extract::Path(version): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    let info = state
        .storage()
        .get_account_data(&user_id, None, &format!("_maelstrom.key_backup.{version}"))
        .await
        .map_err(|_| MatrixError::not_found("Key backup version not found"))?;

    let mut response = info;
    response["version"] = serde_json::Value::String(version);
    response["count"] = serde_json::json!(0);
    response["etag"] = serde_json::Value::String("0".to_string());
    Ok(Json(response))
}

/// PUT /_matrix/client/v3/room_keys/keys/{roomId}/{sessionId}
async fn put_room_key(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::extract::Path((room_id, session_id)): axum::extract::Path<(String, String)>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();

    // Get current backup version
    let ver = state
        .storage()
        .get_account_data(&user_id, None, "_maelstrom.key_backup_version")
        .await
        .ok()
        .and_then(|v| v.get("version").and_then(|n| n.as_u64()))
        .unwrap_or(0)
        .to_string();

    let key = format!("_maelstrom.room_key.{ver}.{room_id}.{session_id}");

    // Apply replacement rules per spec: only replace if new key is "better"
    let should_replace =
        if let Ok(existing) = state.storage().get_account_data(&user_id, None, &key).await {
            let old_verified = existing
                .get("is_verified")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let new_verified = body
                .get("is_verified")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let old_fmi = existing
                .get("first_message_index")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            let new_fmi = body
                .get("first_message_index")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            let old_fc = existing
                .get("forwarded_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            let new_fc = body
                .get("forwarded_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);

            // New key wins if: verified beats unverified, or lower first_message_index, or lower forwarded_count
            if new_verified && !old_verified {
                true
            } else if !new_verified && old_verified {
                false
            } else if new_fmi < old_fmi {
                true
            } else if new_fmi > old_fmi {
                false
            } else {
                new_fc < old_fc
            }
        } else {
            true // No existing key
        };

    if should_replace {
        let _ = state
            .storage()
            .set_account_data(&user_id, None, &key, &body)
            .await;
    }

    Ok(Json(serde_json::json!({
        "count": 1,
        "etag": "1",
    })))
}

/// GET /_matrix/client/v3/room_keys/keys/{roomId}/{sessionId}
async fn get_room_key(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    axum::extract::Path((room_id, session_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let user_id = auth.user_id.to_string();
    let ver = state
        .storage()
        .get_account_data(&user_id, None, "_maelstrom.key_backup_version")
        .await
        .ok()
        .and_then(|v| v.get("version").and_then(|n| n.as_u64()))
        .unwrap_or(0)
        .to_string();

    let key = format!("_maelstrom.room_key.{ver}.{room_id}.{session_id}");
    let data = state
        .storage()
        .get_account_data(&user_id, None, &key)
        .await
        .map_err(|_| MatrixError::not_found("Key not found"))?;

    Ok(Json(data))
}

/// PUT /_matrix/client/v3/room_keys/keys/{roomId}
async fn put_room_keys(
    State(_state): State<AppState>,
    _auth: AuthenticatedUser,
    axum::extract::Path(_room_id): axum::extract::Path<String>,
    axum::Json(_body): axum::Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"count": 0, "etag": "0"}))
}

/// GET /_matrix/client/v3/room_keys/keys/{roomId}
async fn get_room_keys(
    _auth: AuthenticatedUser,
    axum::extract::Path(_room_id): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"sessions": {}}))
}

/// PUT /_matrix/client/v3/room_keys/keys
async fn put_all_room_keys(
    _auth: AuthenticatedUser,
    axum::Json(_body): axum::Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({"count": 0, "etag": "0"}))
}

/// GET /_matrix/client/v3/room_keys/keys
async fn get_all_room_keys(_auth: AuthenticatedUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({"rooms": {}}))
}
