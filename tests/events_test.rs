mod common;

use http::StatusCode;

/// Helper: create a room and return its ID.
async fn create_room(router: &axum::Router, token: &str) -> String {
    let (_, resp) = common::post_json_authed(
        router,
        "/_matrix/client/v3/createRoom",
        &serde_json::json!({}),
        token,
    )
    .await;
    serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn test_send_message() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "sender", "pass").await;
    let room_id = create_room(&router, &token).await;

    let (status, resp) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn1"),
        &serde_json::json!({"msgtype": "m.text", "body": "Hello!"}),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "send failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json["event_id"].as_str().unwrap().starts_with('$'));
}

#[tokio::test]
async fn test_send_message_txn_id_dedup() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "deduper", "pass").await;
    let room_id = create_room(&router, &token).await;

    let body = serde_json::json!({"msgtype": "m.text", "body": "Hello!"});

    let (_, resp1) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn_dup"),
        &body,
        &token,
    )
    .await;
    let (_, resp2) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn_dup"),
        &body,
        &token,
    )
    .await;

    let eid1 = serde_json::from_str::<serde_json::Value>(&resp1).unwrap()["event_id"]
        .as_str()
        .unwrap()
        .to_string();
    let eid2 = serde_json::from_str::<serde_json::Value>(&resp2).unwrap()["event_id"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(eid1, eid2, "duplicate txn_id should return same event_id");
}

#[tokio::test]
async fn test_get_event() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "getter", "pass").await;
    let room_id = create_room(&router, &token).await;

    let (_, resp) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn_get"),
        &serde_json::json!({"msgtype": "m.text", "body": "Get me"}),
        &token,
    )
    .await;
    let event_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["event_id"]
        .as_str()
        .unwrap()
        .to_string();

    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/event/{event_id}"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get event failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["type"], "m.room.message");
    assert_eq!(json["content"]["body"], "Get me");
}

#[tokio::test]
async fn test_get_messages() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "msglist", "pass").await;
    let room_id = create_room(&router, &token).await;

    // Send 3 messages
    for i in 0..3 {
        common::put_json_authed(
            &router,
            &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn{i}"),
            &serde_json::json!({"msgtype": "m.text", "body": format!("msg {i}")}),
            &token,
        )
        .await;
    }

    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/messages?dir=b&limit=10"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "messages failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let chunk = json["chunk"].as_array().unwrap();
    assert!(
        chunk.len() >= 3,
        "expected at least 3 messages, got {}",
        chunk.len()
    );
}

#[tokio::test]
async fn test_get_room_state() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "stater", "pass").await;
    let room_id = create_room(&router, &token).await;

    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/state"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get state failed: {resp}");

    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap();
    // Should have at least m.room.create, m.room.member, m.room.power_levels
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert!(types.contains(&"m.room.create"), "missing m.room.create");
    assert!(types.contains(&"m.room.member"), "missing m.room.member");
    assert!(
        types.contains(&"m.room.power_levels"),
        "missing m.room.power_levels"
    );
}

#[tokio::test]
async fn test_set_state_event() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "statesetter", "pass").await;
    let room_id = create_room(&router, &token).await;

    // Set room name via state endpoint
    let (status, _) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/state/m.room.name"),
        &serde_json::json!({"name": "New Name"}),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Get it back
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/state/m.room.name"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["name"], "New Name");
}
