mod common;

use http::StatusCode;

#[tokio::test]
async fn test_get_displayname_empty() {
    let router = common::test_router();
    common::register_user(&router, "noname", "pass").await;

    let (status, resp) = common::get(
        &router,
        "/_matrix/client/v3/profile/@noname:localhost/displayname",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    // displayname should be absent or null when not set
    assert!(json.get("displayname").is_none() || json["displayname"].is_null());
}

#[tokio::test]
async fn test_set_and_get_displayname() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "nametest", "pass").await;

    // Set display name
    let body = serde_json::json!({ "displayname": "Test User" });
    let (status, _) = common::put_json_authed(
        &router,
        "/_matrix/client/v3/profile/@nametest:localhost/displayname",
        &body,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Get display name
    let (status, resp) = common::get(
        &router,
        "/_matrix/client/v3/profile/@nametest:localhost/displayname",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["displayname"], "Test User");
}

#[tokio::test]
async fn test_set_and_get_avatar_url() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "avatartest", "pass").await;

    // Set avatar URL
    let body = serde_json::json!({ "avatar_url": "mxc://localhost/abc123" });
    let (status, _) = common::put_json_authed(
        &router,
        "/_matrix/client/v3/profile/@avatartest:localhost/avatar_url",
        &body,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Get avatar URL
    let (status, resp) = common::get(
        &router,
        "/_matrix/client/v3/profile/@avatartest:localhost/avatar_url",
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["avatar_url"], "mxc://localhost/abc123");
}

#[tokio::test]
async fn test_get_full_profile() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "fullprofile", "pass").await;

    // Set both
    let body = serde_json::json!({ "displayname": "Full Profile" });
    common::put_json_authed(
        &router,
        "/_matrix/client/v3/profile/@fullprofile:localhost/displayname",
        &body,
        &token,
    )
    .await;

    let body = serde_json::json!({ "avatar_url": "mxc://localhost/avatar" });
    common::put_json_authed(
        &router,
        "/_matrix/client/v3/profile/@fullprofile:localhost/avatar_url",
        &body,
        &token,
    )
    .await;

    // Get combined profile
    let (status, resp) =
        common::get(&router, "/_matrix/client/v3/profile/@fullprofile:localhost").await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["displayname"], "Full Profile");
    assert_eq!(json["avatar_url"], "mxc://localhost/avatar");
}

#[tokio::test]
async fn test_cannot_set_other_users_displayname() {
    let router = common::test_router();
    let (token_alice, _, _) = common::register_user(&router, "alice", "pass").await;
    common::register_user(&router, "bob", "pass").await;

    // Alice tries to set Bob's display name
    let body = serde_json::json!({ "displayname": "Hacked" });
    let (status, resp) = common::put_json_authed(
        &router,
        "/_matrix/client/v3/profile/@bob:localhost/displayname",
        &body,
        &token_alice,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_FORBIDDEN");
}

#[tokio::test]
async fn test_displayname_change_propagates_to_room_member_event() {
    let router = common::test_router();
    let (token, user_id, _) = common::register_user(&router, "proptest", "pass").await;

    // Create a room
    let body = serde_json::json!({ "preset": "public_chat" });
    let (status, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;
    assert_eq!(status, StatusCode::OK);
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Do an initial sync to get a since token
    let (status, resp) = common::get_authed(&router, "/_matrix/client/v3/sync", &token).await;
    assert_eq!(status, StatusCode::OK);
    let since = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["next_batch"]
        .as_str()
        .unwrap()
        .to_string();

    // Update display name
    let body = serde_json::json!({ "displayname": "New Name" });
    let (status, _) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/profile/{user_id}/displayname"),
        &body,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Incremental sync should show the m.room.member event with updated displayname
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/sync?since={since}"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let room = &json["rooms"]["join"][&room_id];
    let timeline_events = room["timeline"]["events"].as_array().unwrap();

    // Find the m.room.member event with updated displayname
    let member_event = timeline_events.iter().find(|e| {
        e["type"] == "m.room.member"
            && e["state_key"] == user_id
            && e["content"]["displayname"] == "New Name"
    });
    assert!(
        member_event.is_some(),
        "Expected m.room.member event with updated displayname in sync response"
    );
}

#[tokio::test]
async fn test_get_profile_nonexistent_user() {
    let router = common::test_router();
    let (status, resp) = common::get(
        &router,
        "/_matrix/client/v3/profile/@nobody:localhost/displayname",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_NOT_FOUND");
}
