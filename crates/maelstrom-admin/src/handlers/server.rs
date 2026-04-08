use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use maelstrom_core::error::MatrixError;
use sysinfo::System;

use crate::AdminState;
use crate::auth::AdminUser;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/_maelstrom/admin/v1/server/info", get(server_info))
        .route("/_maelstrom/admin/v1/server/health", get(health_detailed))
        .route("/_maelstrom/admin/v1/metrics", get(prometheus_metrics))
}

async fn server_info(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let mut sys = System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();

    let uptime = state.uptime_secs();
    let hours = uptime / 3600;
    let mins = (uptime % 3600) / 60;

    Ok(Json(serde_json::json!({
        "server_name": state.server_name().as_str(),
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime,
        "uptime_human": format!("{hours}h {mins}m"),
        "system": {
            "total_memory_mb": sys.total_memory() / 1_048_576,
            "used_memory_mb": sys.used_memory() / 1_048_576,
            "cpu_count": sys.cpus().len(),
        },
        "database": {
            "healthy": state.storage().is_healthy().await,
        },
    })))
}

async fn health_detailed(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let db_healthy = state.storage().is_healthy().await;

    Ok(Json(serde_json::json!({
        "status": if db_healthy { "healthy" } else { "degraded" },
        "services": {
            "database": { "status": if db_healthy { "up" } else { "down" } },
        },
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

/// GET /_maelstrom/admin/v1/metrics — Prometheus-compatible metrics.
async fn prometheus_metrics(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<String, MatrixError> {
    let mut sys = System::new();
    sys.refresh_memory();

    let uptime = state.uptime_secs();
    let db_healthy = state.storage().is_healthy().await;

    let metrics = format!(
        "# HELP maelstrom_uptime_seconds Server uptime in seconds\n\
         # TYPE maelstrom_uptime_seconds gauge\n\
         maelstrom_uptime_seconds {uptime}\n\
         # HELP maelstrom_memory_used_bytes Used memory in bytes\n\
         # TYPE maelstrom_memory_used_bytes gauge\n\
         maelstrom_memory_used_bytes {}\n\
         # HELP maelstrom_memory_total_bytes Total memory in bytes\n\
         # TYPE maelstrom_memory_total_bytes gauge\n\
         maelstrom_memory_total_bytes {}\n\
         # HELP maelstrom_database_up Database connectivity (1=up, 0=down)\n\
         # TYPE maelstrom_database_up gauge\n\
         maelstrom_database_up {}\n",
        sys.used_memory(),
        sys.total_memory(),
        if db_healthy { 1 } else { 0 },
    );

    Ok(metrics)
}
