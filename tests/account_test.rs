mod common;

use http::StatusCode;

#[tokio::test]
async fn test_whoami() {
    let router = common::test_router();
    let (token, user_id, device_id) = common::register_user(&router, "whoamitest", "pass").await;

    let (status, resp) =
        common::get_authed(&router, "/_matrix/client/v3/account/whoami", &token).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["user_id"], user_id);
    assert_eq!(json["device_id"], device_id);
    assert_eq!(json["is_guest"], false);
}

#[tokio::test]
async fn test_whoami_no_token() {
    let router = common::test_router();
    let (status, resp) = common::get(&router, "/_matrix/client/v3/account/whoami").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_MISSING_TOKEN");
}

#[tokio::test]
async fn test_whoami_invalid_token() {
    let router = common::test_router();
    let (status, resp) = common::get_authed(
        &router,
        "/_matrix/client/v3/account/whoami",
        "mat_invalidtoken",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_UNKNOWN_TOKEN");
}

#[tokio::test]
async fn test_whoami_with_query_param_token() {
    let router = common::test_router();
    let (token, user_id, _) = common::register_user(&router, "queryparam", "pass").await;

    let uri = format!("/_matrix/client/v3/account/whoami?access_token={token}");
    let (status, resp) = common::get(&router, &uri).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["user_id"], user_id);
}

#[tokio::test]
async fn test_change_password() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "changepw", "oldpass").await;

    let body = serde_json::json!({
        "new_password": "newpass",
        "auth": { "type": "m.login.dummy" },
    });

    let (status, _) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/account/password",
        &body,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Login with new password should work
    let login_body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "changepw" },
        "password": "newpass",
    });
    let (status, _) = common::post_json(&router, "/_matrix/client/v3/login", &login_body).await;
    assert_eq!(status, StatusCode::OK);

    // Login with old password should fail
    let login_body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "changepw" },
        "password": "oldpass",
    });
    let (status, _) = common::post_json(&router, "/_matrix/client/v3/login", &login_body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_deactivate_account() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "deactivateme", "pass").await;

    let body = serde_json::json!({
        "auth": { "type": "m.login.dummy" },
    });

    let (status, _) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/account/deactivate",
        &body,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Token should be invalid (all devices removed)
    let (status, _) =
        common::get_authed(&router, "/_matrix/client/v3/account/whoami", &token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Login should fail (account deactivated)
    let login_body = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": "deactivateme" },
        "password": "pass",
    });
    let (status, resp) = common::post_json(&router, "/_matrix/client/v3/login", &login_body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_USER_DEACTIVATED");
}
