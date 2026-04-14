//! Well-known server discovery.
//!
//! The `.well-known` endpoints enable automatic server discovery. When a user
//! enters their Matrix ID (e.g. `@alice:example.com`), the client extracts
//! the server name and fetches `/.well-known/matrix/client` from that domain
//! to find the actual homeserver base URL.
//!
//! Similarly, other homeservers use `/.well-known/matrix/server` during
//! federation to discover the correct host and port to connect to.
//!
//! These endpoints do **not** require authentication.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/.well-known/matrix/client` | Client-side server discovery (homeserver base URL) |
//! | `GET` | `/.well-known/matrix/server` | Server-side (federation) discovery |
//!
//! # Matrix spec
//!
//! * [Server discovery](https://spec.matrix.org/v1.18/client-server-api/#server-discovery)

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Serialize;

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/.well-known/matrix/client", get(get_wellknown))
        .route("/.well-known/matrix/server", get(get_server_wellknown))
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

/// `GET /.well-known/matrix/server` — federation server discovery.
/// Returns `{"m.server": "hostname:port"}` so remote servers know where to connect.
async fn get_server_wellknown(State(state): State<AppState>) -> impl IntoResponse {
    // Use server_name with port 8448 as the federation endpoint
    let server_name = state.server_name().as_str();
    let m_server = if server_name.contains(':') {
        server_name.to_string()
    } else {
        format!("{server_name}:8448")
    };
    Json(serde_json::json!({ "m.server": m_server }))
}
