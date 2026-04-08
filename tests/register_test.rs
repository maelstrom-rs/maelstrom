mod common;

use http::StatusCode;

#[tokio::test]
async fn test_register_with_dummy_auth() {
    let router = common::test_router();
    let body = serde_json::json!({
        "auth": { "type": "m.login.dummy" },
        "username": "alice",
        "password": "secret123",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["user_id"], "@alice:localhost");
    assert!(json["access_token"].as_str().is_some());
    assert!(json["device_id"].as_str().is_some());
}

#[tokio::test]
async fn test_register_returns_uia_without_auth() {
    let router = common::test_router();
    let body = serde_json::json!({
        "username": "bob",
        "password": "secret123",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json["flows"].is_array());
    assert!(json["session"].is_string());
}

#[tokio::test]
async fn test_register_duplicate_username() {
    let router = common::test_router();
    let body = serde_json::json!({
        "auth": { "type": "m.login.dummy" },
        "username": "alice",
        "password": "secret123",
    });

    // First registration succeeds
    let (status, _) = common::post_json(&router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, StatusCode::OK);

    // Second registration fails
    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_USER_IN_USE");
}

#[tokio::test]
async fn test_register_invalid_username() {
    let router = common::test_router();
    let body = serde_json::json!({
        "auth": { "type": "m.login.dummy" },
        "username": "Alice With Spaces",
        "password": "secret123",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_INVALID_USERNAME");
}

#[tokio::test]
async fn test_register_inhibit_login() {
    let router = common::test_router();
    let body = serde_json::json!({
        "auth": { "type": "m.login.dummy" },
        "username": "nologin",
        "password": "secret123",
        "inhibit_login": true,
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/register", &body).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["user_id"], "@nologin:localhost");
    assert!(json.get("access_token").is_none());
    assert!(json.get("device_id").is_none());
}

#[tokio::test]
async fn test_register_available_true() {
    let router = common::test_router();
    let (status, resp) =
        common::get(&router, "/_matrix/client/v3/register/available?username=newuser").await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["available"], true);
}

#[tokio::test]
async fn test_register_available_taken() {
    let router = common::test_router();

    // Register a user first
    common::register_user(&router, "taken", "pass").await;

    let (status, resp) =
        common::get(&router, "/_matrix/client/v3/register/available?username=taken").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_USER_IN_USE");
}

#[tokio::test]
async fn test_register_available_invalid_username() {
    let router = common::test_router();
    let (status, resp) =
        common::get(&router, "/_matrix/client/v3/register/available?username=BAD%20NAME").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_INVALID_USERNAME");
}
