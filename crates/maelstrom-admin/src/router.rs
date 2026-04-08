use axum::Router;
use tower_http::services::ServeDir;

use crate::handlers;
use crate::AdminState;

/// Build the complete admin router with API + dashboard + static files.
pub fn build(state: AdminState) -> Router {
    let admin_api = Router::new()
        // JSON API endpoints
        .merge(handlers::users::routes())
        .merge(handlers::rooms::routes())
        .merge(handlers::media::routes())
        .merge(handlers::federation::routes())
        .merge(handlers::server::routes())
        .merge(handlers::reports::routes())
        // SSR dashboard pages
        .merge(handlers::dashboard::routes());

    // Static files (CSS)
    let static_dir = ServeDir::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/static")
    );

    Router::new()
        .merge(admin_api)
        .nest_service("/_maelstrom/admin/static", static_dir)
        .with_state(state)
}
