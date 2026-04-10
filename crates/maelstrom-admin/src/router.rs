//! Admin router assembly and static file serving.
//!
//! Builds the complete admin [`Router`] by merging two groups of routes:
//!
//! **JSON API routes** (all require admin auth):
//! - `/_maelstrom/admin/v1/users/*`       -- user management (list, get, deactivate, admin flag, reset password)
//! - `/_maelstrom/admin/v1/rooms/*`       -- room inspection and shutdown
//! - `/_maelstrom/admin/v1/media/*`       -- per-user media listing, quarantine, retention config
//! - `/_maelstrom/admin/v1/federation/*`  -- federation signing-key stats
//! - `/_maelstrom/admin/v1/server/*`      -- server info, detailed health, Prometheus metrics
//! - `/_maelstrom/admin/v1/reports`       -- content-report review
//!
//! **SSR dashboard pages** (HTML, also require admin auth):
//! - `/_maelstrom/admin/`                 -- overview dashboard (uptime, memory, DB status)
//! - `/_maelstrom/admin/users`            -- user management page
//! - `/_maelstrom/admin/rooms`            -- room management page
//! - `/_maelstrom/admin/federation`       -- federation status page
//!
//! Static CSS and JS assets are served from `/_maelstrom/admin/static/` via
//! [`tower_http::services::ServeDir`], pointing at the `static/` directory
//! embedded relative to the crate's `CARGO_MANIFEST_DIR`.

use axum::Router;
use tower_http::services::ServeDir;

use crate::AdminState;
use crate::handlers;

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
    let static_dir = ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/static"));

    Router::new()
        .merge(admin_api)
        .nest_service("/_maelstrom/admin/static", static_dir)
        .with_state(state)
}
