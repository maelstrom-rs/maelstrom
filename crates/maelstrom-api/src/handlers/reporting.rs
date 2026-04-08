use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;
use maelstrom_storage::traits::ReportRecord;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v3/rooms/{roomId}/report/{eventId}",
        post(report_event),
    )
}

#[derive(Deserialize)]
struct ReportParams {
    #[serde(rename = "roomId")]
    room_id: String,
    #[serde(rename = "eventId")]
    event_id: String,
}

#[derive(Deserialize)]
struct ReportRequest {
    #[serde(default)]
    reason: Option<String>,
    #[serde(default = "default_score")]
    score: i64,
}

fn default_score() -> i64 {
    -100
}

/// POST /rooms/{roomId}/report/{eventId} — report an event for abuse.
async fn report_event(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Path(params): Path<ReportParams>,
    MatrixJson(body): MatrixJson<ReportRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    // Verify event exists
    state
        .storage()
        .get_event(&params.event_id)
        .await
        .map_err(|_| MatrixError::not_found("Event not found"))?;

    let report = ReportRecord {
        event_id: params.event_id,
        room_id: params.room_id,
        reporter: auth.user_id.to_string(),
        reason: body.reason,
        score: body.score.clamp(-100, 0),
    };

    state
        .storage()
        .store_report(&report)
        .await
        .map_err(crate::extractors::storage_error)?;

    Ok(Json(serde_json::json!({})))
}
