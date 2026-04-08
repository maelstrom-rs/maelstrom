use axum::Router;

use crate::FederationState;

/// Build the complete federation router with all server-to-server endpoints.
pub fn build(state: FederationState) -> Router {
    let federation_api = Router::new()
        .merge(crate::key_server::routes())
        .merge(crate::receiver::routes())
        .merge(crate::joins::routes())
        .merge(crate::state::routes())
        .merge(crate::backfill::routes())
        .merge(crate::user_keys::routes());

    Router::new().merge(federation_api).with_state(state)
}
