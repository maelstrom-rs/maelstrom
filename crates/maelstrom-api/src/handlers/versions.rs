//! Supported Matrix spec versions.
//!
//! This is typically the first endpoint a client calls during startup to
//! determine which specification versions and unstable features the server
//! supports. The response guides the client on which API paths and behaviours
//! are available.
//!
//! This endpoint does **not** require authentication.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/_matrix/client/versions` | List supported spec versions and unstable feature flags |
//!
//! # Matrix spec
//!
//! * [GET /_matrix/client/versions](https://spec.matrix.org/v1.12/client-server-api/#get_matrixclientversions)

use axum::Json;
use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;
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
