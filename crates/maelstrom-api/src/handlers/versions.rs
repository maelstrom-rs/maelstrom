use axum::Router;
use axum::routing::get;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/_matrix/client/versions", get(get_versions))
}

#[derive(Serialize)]
struct VersionsResponse {
    versions: Vec<&'static str>,
    unstable_features: UnstableFeatures,
}

#[derive(Serialize)]
struct UnstableFeatures {
    // Placeholder for MSC flags — populated as features are implemented
}

async fn get_versions() -> impl IntoResponse {
    Json(VersionsResponse {
        versions: vec![
            "v1.1", "v1.2", "v1.3", "v1.4", "v1.5", "v1.6", "v1.7", "v1.8", "v1.9", "v1.10",
            "v1.11", "v1.12",
        ],
        unstable_features: UnstableFeatures {},
    })
}
