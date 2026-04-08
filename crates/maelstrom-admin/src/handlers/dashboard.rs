use askama::Template;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::Router;

use maelstrom_core::error::MatrixError;

use crate::auth::AdminUser;
use crate::templates;
use crate::AdminState;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/_maelstrom/admin/", get(dashboard_page))
        .route("/_maelstrom/admin/users", get(users_page))
        .route("/_maelstrom/admin/rooms", get(rooms_page))
        .route("/_maelstrom/admin/federation", get(federation_page))
}

fn render<T: Template>(tmpl: T) -> Result<Html<String>, MatrixError> {
    tmpl.render()
        .map(Html)
        .map_err(|e| MatrixError::unknown(format!("Template render failed: {e}")))
}

async fn dashboard_page(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Html<String>, MatrixError> {
    let db_healthy = state.storage().is_healthy().await;
    let uptime = state.uptime_secs();
    let hours = uptime / 3600;
    let mins = (uptime % 3600) / 60;

    let mut sys = sysinfo::System::new();
    sys.refresh_memory();

    render(templates::DashboardPage {
        server_name: state.server_name().as_str().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime: format!("{hours}h {mins}m"),
        db_status: if db_healthy { "Healthy" } else { "Down" },
        memory_used_mb: sys.used_memory() / 1_048_576,
        memory_total_mb: sys.total_memory() / 1_048_576,
    })
}

async fn users_page(
    State(_state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Html<String>, MatrixError> {
    render(templates::UsersPage {})
}

async fn rooms_page(
    State(_state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Html<String>, MatrixError> {
    render(templates::RoomsPage {})
}

async fn federation_page(
    State(state): State<AdminState>,
    _admin: AdminUser,
) -> Result<Html<String>, MatrixError> {
    let key_count = state
        .storage()
        .get_active_server_keys()
        .await
        .map(|k| k.len())
        .unwrap_or(0);

    render(templates::FederationPage {
        server_name: state.server_name().as_str().to_string(),
        signing_key_count: key_count,
    })
}
