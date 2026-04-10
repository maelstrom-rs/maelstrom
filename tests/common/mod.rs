#![allow(dead_code)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use http::Request;
use maelstrom_api::notify::LocalNotifier;
use maelstrom_api::router;
use maelstrom_api::state::AppState;
use maelstrom_core::matrix::ephemeral::EphemeralStore;
use maelstrom_core::matrix::id::ServerName;
use maelstrom_storage::mock::MockStorage;
use tower::ServiceExt;

/// Create a test AppState backed by MockStorage + LocalNotifier.
pub fn test_state() -> AppState {
    AppState::new(
        MockStorage::new(),
        LocalNotifier::new(),
        Arc::new(EphemeralStore::new()),
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
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// Send a GET request with Authorization header.
pub async fn get_authed(router: &Router, uri: &str, token: &str) -> (http::StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// Send a POST request with a JSON body.
pub async fn post_json(
    router: &Router,
    uri: &str,
    body: &serde_json::Value,
) -> (http::StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .method("POST")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(body).unwrap()))
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// Send a POST request with a JSON body and Authorization header.
pub async fn post_json_authed(
    router: &Router,
    uri: &str,
    body: &serde_json::Value,
    token: &str,
) -> (http::StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .method("POST")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(body).unwrap()))
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// Send a PUT request with a JSON body and Authorization header.
pub async fn put_json_authed(
    router: &Router,
    uri: &str,
    body: &serde_json::Value,
    token: &str,
) -> (http::StatusCode, String) {
    let req = Request::builder()
        .uri(uri)
        .method("PUT")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_string(body).unwrap()))
        .unwrap();

    let response = router.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// Register a test user and return (access_token, user_id, device_id).
pub async fn register_user(
    router: &Router,
    username: &str,
    password: &str,
) -> (String, String, String) {
    let body = serde_json::json!({
        "auth": { "type": "m.login.dummy" },
        "username": username,
        "password": password,
    });

    let (status, resp) = post_json(router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, http::StatusCode::OK, "Registration failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    (
        json["access_token"].as_str().unwrap().to_string(),
        json["user_id"].as_str().unwrap().to_string(),
        json["device_id"].as_str().unwrap().to_string(),
    )
}
