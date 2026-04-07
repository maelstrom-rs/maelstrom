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
        .merge(handlers::health::routes());

    Router::new()
        .merge(client_api)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}
