mod common;

use http::StatusCode;

#[tokio::test]
async fn test_initial_sync_empty() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "syncer", "pass").await;

    let (status, resp) = common::get_authed(&router, "/_matrix/client/v3/sync", &token).await;
    assert_eq!(status, StatusCode::OK, "sync failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json["next_batch"].as_str().is_some());
    assert!(json["rooms"]["join"].is_object());
}

#[tokio::test]
async fn test_initial_sync_with_room() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "syncroom", "pass").await;

    // Create a room
    let (_, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &serde_json::json!({"name": "Sync Test"}),
        &token,
    )
    .await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send a message
    common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn1"),
        &serde_json::json!({"msgtype": "m.text", "body": "Hello sync!"}),
        &token,
    )
    .await;

    // Sync
    let (status, resp) = common::get_authed(&router, "/_matrix/client/v3/sync", &token).await;
    assert_eq!(status, StatusCode::OK, "sync failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();

    // Room should appear in join
    let room_data = &json["rooms"]["join"][&room_id];
    assert!(room_data.is_object(), "room not found in sync response");

    // Should have state events
    let state = room_data["state"]["events"].as_array().unwrap();
    assert!(!state.is_empty(), "state should not be empty");

    // Should have timeline events
    let timeline = room_data["timeline"]["events"].as_array().unwrap();
    assert!(!timeline.is_empty(), "timeline should not be empty");
}

#[tokio::test]
async fn test_incremental_sync() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "incsync", "pass").await;

    // Create a room
    let (_, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &serde_json::json!({}),
        &token,
    )
    .await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Initial sync to get a token
    let (_, resp) = common::get_authed(&router, "/_matrix/client/v3/sync", &token).await;
    let next_batch = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["next_batch"]
        .as_str()
        .unwrap()
        .to_string();

    // Send a message after the sync token
    common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn_inc"),
        &serde_json::json!({"msgtype": "m.text", "body": "New message!"}),
        &token,
    )
    .await;

    // Incremental sync
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/sync?since={next_batch}"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "incremental sync failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();

    // Should see the new message in timeline
    let room_data = &json["rooms"]["join"][&room_id];
    assert!(room_data.is_object(), "room not found in incremental sync");

    let timeline = room_data["timeline"]["events"].as_array().unwrap();
    assert!(
        timeline
            .iter()
            .any(|e| { e["type"] == "m.room.message" && e["content"]["body"] == "New message!" }),
        "new message not found in incremental sync timeline"
    );
}

#[tokio::test]
async fn test_sync_no_token_returns_unauthorized() {
    let router = common::test_router();
    let (status, _) = common::get(&router, "/_matrix/client/v3/sync").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
