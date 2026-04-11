//! Application Service API endpoints.
//!
//! Implements both admin endpoints for managing AS registrations and
//! the third-party protocol lookup endpoints that clients use to
//! discover bridged networks.
//!
//! # Admin endpoints (Maelstrom-specific)
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST`   | `/_maelstrom/admin/v1/appservice`       | Register an AS from JSON |
//! | `GET`    | `/_maelstrom/admin/v1/appservices`       | List registered ASes |
//! | `DELETE` | `/_maelstrom/admin/v1/appservice/{asId}` | Unregister an AS |
//!
//! # Third-party protocol endpoints (Matrix spec)
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/_matrix/client/v3/thirdparty/protocols`            | List protocols from all ASes |
//! | `GET` | `/_matrix/client/v3/thirdparty/protocol/{protocol}`  | Protocol details |
//! | `GET` | `/_matrix/client/v3/thirdparty/location/{protocol}`  | Search locations by protocol |
//! | `GET` | `/_matrix/client/v3/thirdparty/user/{protocol}`      | Search users by protocol |
//! | `GET` | `/_matrix/client/v3/thirdparty/location`             | Search all locations |
//! | `GET` | `/_matrix/client/v3/thirdparty/user`                 | Search all users |
//!
//! # Event push
//!
//! The [`notify_appservices`] function checks whether a newly stored event
//! matches any registered AS's user namespace and pushes it via HTTP PUT
//! to `/_matrix/app/v1/transactions/{txnId}` on the AS's URL.  Currently
//! this is called synchronously after event storage; a production deployment
//! would use a background queue.

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::event::Pdu;
use maelstrom_storage::traits::{AppServiceRecord, Storage};
use serde_json::Value;

use crate::state::AppState;

/// Build the router for application service endpoints.
pub fn routes() -> Router<AppState> {
    Router::new()
        // Admin endpoints
        .route("/_maelstrom/admin/v1/appservice", post(register_appservice))
        .route("/_maelstrom/admin/v1/appservices", get(list_appservices))
        .route(
            "/_maelstrom/admin/v1/appservice/{asId}",
            delete(delete_appservice),
        )
        // Third-party protocol endpoints
        .route(
            "/_matrix/client/v3/thirdparty/protocols",
            get(get_protocols),
        )
        .route(
            "/_matrix/client/v3/thirdparty/protocol/{protocol}",
            get(get_protocol),
        )
        .route(
            "/_matrix/client/v3/thirdparty/location/{protocol}",
            get(get_location_by_protocol),
        )
        .route(
            "/_matrix/client/v3/thirdparty/user/{protocol}",
            get(get_user_by_protocol),
        )
        .route("/_matrix/client/v3/thirdparty/location", get(get_locations))
        .route("/_matrix/client/v3/thirdparty/user", get(get_users))
}

// ---------------------------------------------------------------------------
// Admin endpoints
// ---------------------------------------------------------------------------

/// Register a new application service.
///
/// Accepts a JSON body matching [`AppServiceRecord`].  The `id` field must
/// be unique across all registered ASes.
async fn register_appservice(
    State(state): State<AppState>,
    Json(record): Json<AppServiceRecord>,
) -> Result<Json<Value>, MatrixError> {
    state
        .storage()
        .register_appservice(&record)
        .await
        .map_err(|e| {
            tracing::error!("Failed to register appservice: {e}");
            MatrixError::unknown(format!("Failed to register appservice: {e}"))
        })?;

    Ok(Json(serde_json::json!({ "id": record.id })))
}

/// List all registered application services.
async fn list_appservices(State(state): State<AppState>) -> Result<Json<Value>, MatrixError> {
    let appservices = state
        .storage()
        .list_appservices()
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to list appservices: {e}")))?;

    Ok(Json(serde_json::json!({ "appservices": appservices })))
}

/// Unregister an application service by ID.
async fn delete_appservice(
    State(state): State<AppState>,
    Path(as_id): Path<String>,
) -> Result<Json<Value>, MatrixError> {
    state
        .storage()
        .delete_appservice(&as_id)
        .await
        .map_err(|e| MatrixError::unknown(format!("Failed to delete appservice: {e}")))?;

    Ok(Json(serde_json::json!({})))
}

// ---------------------------------------------------------------------------
// Third-party protocol endpoints
// ---------------------------------------------------------------------------

/// List all third-party protocols advertised by registered ASes.
///
/// Returns a map of protocol name to protocol metadata.  Currently the
/// metadata is minimal (empty fields/instances); full delegation to the
/// AS would require an HTTP call to the AS's URL.
async fn get_protocols(State(state): State<AppState>) -> Result<Json<Value>, MatrixError> {
    let appservices = state.storage().list_appservices().await.unwrap_or_default();
    let mut protocols = serde_json::Map::new();
    for as_record in &appservices {
        for proto in &as_record.protocols {
            protocols.insert(
                proto.clone(),
                serde_json::json!({
                    "user_fields": [],
                    "location_fields": [],
                    "icon": "",
                    "field_types": {},
                    "instances": []
                }),
            );
        }
    }
    Ok(Json(Value::Object(protocols)))
}

/// Get details for a specific third-party protocol.
async fn get_protocol(
    State(state): State<AppState>,
    Path(protocol): Path<String>,
) -> Result<Json<Value>, MatrixError> {
    let appservices = state.storage().list_appservices().await.unwrap_or_default();
    let has_protocol = appservices.iter().any(|a| a.protocols.contains(&protocol));

    if !has_protocol {
        return Err(MatrixError::not_found("Protocol not found"));
    }

    Ok(Json(serde_json::json!({
        "user_fields": [],
        "location_fields": [],
        "icon": "",
        "field_types": {},
        "instances": []
    })))
}

/// Search for third-party locations by protocol.
async fn get_location_by_protocol(
    State(_state): State<AppState>,
    Path(_protocol): Path<String>,
) -> Result<Json<Value>, MatrixError> {
    // Full implementation would query the AS via HTTP.
    Ok(Json(serde_json::json!([])))
}

/// Search for third-party users by protocol.
async fn get_user_by_protocol(
    State(_state): State<AppState>,
    Path(_protocol): Path<String>,
) -> Result<Json<Value>, MatrixError> {
    Ok(Json(serde_json::json!([])))
}

/// Search all third-party locations across all protocols.
async fn get_locations(State(_state): State<AppState>) -> Result<Json<Value>, MatrixError> {
    Ok(Json(serde_json::json!([])))
}

/// Search all third-party users across all protocols.
async fn get_users(State(_state): State<AppState>) -> Result<Json<Value>, MatrixError> {
    Ok(Json(serde_json::json!([])))
}

// ---------------------------------------------------------------------------
// Event push to application services
// ---------------------------------------------------------------------------

/// Check if any registered application service should receive this event
/// and push it via HTTP PUT to the AS's transaction endpoint.
///
/// Called synchronously after storing events.  A production implementation
/// would use a background queue to avoid blocking the request path.
pub async fn notify_appservices(storage: &dyn Storage, event: &Pdu, http_client: &reqwest::Client) {
    let appservices = match storage.list_appservices().await {
        Ok(list) => list,
        Err(_) => return,
    };

    for as_record in appservices {
        // Check if event sender matches any user namespace regex
        let matches = as_record.user_namespaces.iter().any(|ns| {
            regex::Regex::new(&ns.regex)
                .map(|re| re.is_match(&event.sender))
                .unwrap_or(false)
        });

        if matches {
            let txn_id = format!("{}_{}", event.stream_position, event.event_id);
            let url = format!("{}/_matrix/app/v1/transactions/{}", as_record.url, txn_id);
            let body = serde_json::json!({
                "events": [event.to_federation_json()]
            });

            if let Err(e) = http_client
                .put(&url)
                .header("Authorization", format!("Bearer {}", as_record.hs_token))
                .json(&body)
                .send()
                .await
            {
                tracing::warn!(
                    as_id = %as_record.id,
                    event_id = %event.event_id,
                    "Failed to push event to appservice: {e}"
                );
            }
        }
    }
}
