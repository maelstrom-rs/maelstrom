use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use http::StatusCode;
use serde::Serialize;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/_health/live", get(liveness))
        .route("/_health/ready", get(readiness))
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

async fn liveness() -> impl IntoResponse {
    Json(HealthResponse { status: "ok" })
}

async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let db_healthy = state.storage().is_healthy().await;

    if db_healthy {
        (StatusCode::OK, Json(HealthResponse { status: "ok" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "unavailable",
            }),
        )
    }
}
