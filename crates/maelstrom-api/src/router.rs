use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::handlers;
use crate::state::AppState;

/// Build the complete Axum router with all middleware and routes.
pub fn build(state: AppState) -> Router {
    let client_api = Router::new()
        .merge(handlers::versions::routes())
        .merge(handlers::wellknown::routes())
        .merge(handlers::health::routes())
        .merge(handlers::register::routes())
        .merge(handlers::auth::routes())
        .merge(handlers::capabilities::routes())
        .merge(handlers::account::routes())
        .merge(handlers::profile::routes())
        .merge(handlers::rooms::routes())
        .merge(handlers::directory::routes())
        .merge(handlers::events::routes())
        .merge(handlers::search::routes())
        .merge(handlers::sync::routes())
        .merge(handlers::typing::routes())
        .merge(handlers::receipts::routes())
        .merge(handlers::presence::routes())
        .merge(handlers::keys::routes())
        .merge(handlers::to_device::routes())
        .merge(handlers::media::routes())
        .merge(handlers::relations::routes())
        .merge(handlers::threads::routes())
        .merge(handlers::spaces::routes())
        .merge(handlers::knock::routes())
        .merge(handlers::reporting::routes());

    Router::new()
        .merge(client_api)
        .layer(axum::middleware::from_fn(crate::middleware::rate_limit::rate_limit_auth))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
