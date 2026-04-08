mod common;

use http::StatusCode;

#[tokio::test]
async fn test_login_flows_returns_password() {
    let router = common::test_router();
    let (status, resp) = common::get(&router, "/_matrix/client/v3/login").await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let flows = json["flows"].as_array().unwrap();
    assert!(flows.iter().any(|f| f["type"] == "m.login.password"));
}

#[tokio::test]
async fn test_login_success() {
    let router = common::test_router();

    // Register first
    common::register_user(&router, "logintest", "mypassword").await;

    // Login
    let body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "logintest" },
        "password": "mypassword",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/login", &body).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["user_id"], "@logintest:localhost");
    assert!(json["access_token"].as_str().is_some());
    assert!(json["device_id"].as_str().is_some());
    assert_eq!(json["home_server"], "localhost");
}

#[tokio::test]
async fn test_login_with_full_user_id() {
    let router = common::test_router();
    common::register_user(&router, "fullid", "pass").await;

    let body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "@fullid:localhost" },
        "password": "pass",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/login", &body).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["user_id"], "@fullid:localhost");
}

#[tokio::test]
async fn test_login_wrong_password() {
    let router = common::test_router();
    common::register_user(&router, "wrongpw", "correctpass").await;

    let body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "wrongpw" },
        "password": "wrongpass",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/login", &body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_FORBIDDEN");
}

#[tokio::test]
async fn test_login_nonexistent_user() {
    let router = common::test_router();

    let body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "nobody" },
        "password": "anything",
    });

    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/login", &body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_FORBIDDEN");
}

#[tokio::test]
async fn test_logout() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "logouttest", "pass").await;

    // Logout
    let (status, _) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/logout",
        &serde_json::json!({}),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Token should no longer work
    let (status, resp) =
        common::get_authed(&router, "/_matrix/client/v3/account/whoami", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_UNKNOWN_TOKEN");
}

#[tokio::test]
async fn test_logout_all() {
    let router = common::test_router();
    let (token1, _, _) = common::register_user(&router, "logoutall", "pass").await;

    // Login again to get a second device
    let body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "logoutall" },
        "password": "pass",
    });
    let (_, resp) = common::post_json(&router, "/_matrix/client/v3/login", &body).await;
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let token2 = json["access_token"].as_str().unwrap();

    // Logout all using token1
    let (status, _) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/logout/all",
        &serde_json::json!({}),
        &token1,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Both tokens should be invalid
    let (status, _) =
        common::get_authed(&router, "/_matrix/client/v3/account/whoami", &token1).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) =
        common::get_authed(&router, "/_matrix/client/v3/account/whoami", token2).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
