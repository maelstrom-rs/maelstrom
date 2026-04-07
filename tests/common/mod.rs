use axum::Router;
use axum::body::Body;
use http::Request;
use maelstrom_api::router;
use maelstrom_api::state::AppState;
use maelstrom_core::identifiers::ServerName;
use maelstrom_storage::mock::MockStorage;
use tower::ServiceExt;

/// Create a test AppState backed by MockStorage.
pub fn test_state() -> AppState {
    AppState::new(
        MockStorage::new(),
        ServerName::new("localhost"),
        "http://localhost:8008".to_string(),
    )
}

/// Create a test router with MockStorage.
pub fn test_router() -> Router {
    router::build(test_state())
}

/// Send a GET request to the test router and return the response.
pub async fn get(router: &Router, uri: &str) -> (http::StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(body.to_vec()).unwrap();

    (status, body_str)
}
