//! Health check endpoints.
//!
//! Standard Kubernetes-style health probes for orchestration and load balancers.
//!
//! * **Liveness** (`/_health/live`) -- always returns `200 OK` if the process
//!   is running. Used by Kubernetes liveness probes to decide whether to
//!   restart the container.
//! * **Readiness** (`/_health/ready`) -- returns `200 OK` only if the server
//!   can reach its backing datastore (SurrealDB). Returns `503 Service
//!   Unavailable` otherwise. Used by Kubernetes readiness probes and load
//!   balancers to decide whether to route traffic to this instance.
//!
//! These endpoints do **not** require authentication and are not part of the
//! Matrix specification.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/_health/live` | Liveness probe (always 200 if process is up) |
//! | `GET` | `/_health/ready` | Readiness probe (200 if DB is reachable, 503 otherwise) |

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
