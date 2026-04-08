use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;

use crate::auth::AdminUser;
use crate::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/_maelstrom/admin/v1/media/user/{userId}", get(user_media))
        .route("/_maelstrom/admin/v1/media/{serverName}/{mediaId}/quarantine", post(quarantine_media))
        .route("/_maelstrom/admin/v1/media/{serverName}/{mediaId}/unquarantine", post(unquarantine_media))
        .route("/_maelstrom/admin/v1/media/retention", get(get_retention_config).put(set_retention_config))
        .route("/_maelstrom/admin/v1/media/retention/sweep", post(trigger_retention_sweep))
}

async fn user_media(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let media = state
        .storage()
        .list_user_media(&user_id, 100)
        .await
        .map_err(|e| MatrixError::unknown(format!("{e}")))?;

    let list: Vec<serde_json::Value> = media
        .iter()
        .map(|m| {
            serde_json::json!({
                "media_id": m.media_id,
                "content_type": m.content_type,
                "content_length": m.content_length,
                "filename": m.filename,
                "created_at": m.created_at.to_rfc3339(),
                "quarantined": m.quarantined,
                "mxc_uri": format!("mxc://{}/{}", m.server_name, m.media_id),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "media": list,
        "total": list.len(),
    })))
}

async fn quarantine_media(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path((server_name, media_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    state
        .storage()
        .set_media_quarantined(&server_name, &media_id, true)
        .await
        .map_err(|_| MatrixError::not_found("Media not found"))?;

    Ok(Json(serde_json::json!({"status": "quarantined"})))
}

async fn unquarantine_media(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path((server_name, media_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    state
        .storage()
        .set_media_quarantined(&server_name, &media_id, false)
        .await
        .map_err(|_| MatrixError::not_found("Media not found"))?;

    Ok(Json(serde_json::json!({"status": "unquarantined"})))
}

/// GET /_maelstrom/admin/v1/media/retention — current retention policy.
async fn get_retention_config(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let config = state.retention_config();
    Ok(Json(serde_json::json!({
        "max_age_days": config.max_age_days,
        "sweep_interval_secs": config.sweep_interval_secs,
        "batch_size": config.batch_size,
        "enabled": config.max_age_days > 0,
    })))
}

#[derive(Deserialize)]
struct RetentionConfigUpdate {
    max_age_days: Option<u64>,
    sweep_interval_secs: Option<u64>,
    batch_size: Option<usize>,
}

/// PUT /_maelstrom/admin/v1/media/retention — update retention policy.
async fn set_retention_config(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Json(body): Json<RetentionConfigUpdate>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let mut config = state.retention_config().clone();

    if let Some(days) = body.max_age_days {
        config.max_age_days = days;
    }
    if let Some(secs) = body.sweep_interval_secs {
        config.sweep_interval_secs = secs;
    }
    if let Some(batch) = body.batch_size {
        config.batch_size = batch;
    }

    state.set_retention_config(config.clone());

    tracing::info!(
        max_age_days = config.max_age_days,
        sweep_interval_secs = config.sweep_interval_secs,
        "Retention config updated via admin API"
    );

    Ok(Json(serde_json::json!({
        "status": "updated",
        "max_age_days": config.max_age_days,
        "sweep_interval_secs": config.sweep_interval_secs,
        "batch_size": config.batch_size,
        "enabled": config.max_age_days > 0,
    })))
}

/// POST /_maelstrom/admin/v1/media/retention/sweep — trigger an immediate retention sweep.
async fn trigger_retention_sweep(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let config = state.retention_config();

    if config.max_age_days == 0 {
        return Ok(Json(serde_json::json!({
            "status": "skipped",
            "reason": "Retention disabled (max_age_days = 0)",
        })));
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::days(config.max_age_days as i64);

    let expired = state
        .storage()
        .list_media_before(cutoff, config.batch_size)
        .await
        .map_err(|e| MatrixError::unknown(format!("{e}")))?;

    let count = expired.len();

    // Delete metadata (S3 deletion would need the MediaClient, which is in the API crate)
    for record in &expired {
        let _ = state
            .storage()
            .delete_media(&record.server_name, &record.media_id)
            .await;
    }

    Ok(Json(serde_json::json!({
        "status": "completed",
        "purged": count,
        "cutoff": cutoff.to_rfc3339(),
    })))
}
