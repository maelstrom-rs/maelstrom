use axum::Router;
use axum::extract::State;
use axum::routing::get;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/.well-known/matrix/client", get(get_wellknown))
}

#[derive(Serialize)]
struct WellKnownResponse {
    #[serde(rename = "m.homeserver")]
    homeserver: HomeserverInfo,
}

#[derive(Serialize)]
struct HomeserverInfo {
    base_url: String,
}

async fn get_wellknown(State(state): State<AppState>) -> impl IntoResponse {
    Json(WellKnownResponse {
        homeserver: HomeserverInfo {
            base_url: state.public_base_url().to_string(),
        },
    })
}
