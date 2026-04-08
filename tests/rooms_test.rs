mod common;

use http::StatusCode;

#[tokio::test]
async fn test_create_room_default() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "roomcreator", "pass").await;

    let body = serde_json::json!({});
    let (status, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &body,
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK, "createRoom failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let room_id = json["room_id"].as_str().unwrap();
    assert!(room_id.starts_with('!'));
    assert!(room_id.contains(":localhost"));
}

#[tokio::test]
async fn test_create_room_with_name_and_topic() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "namer", "pass").await;

    let body = serde_json::json!({
        "name": "Test Room",
        "topic": "A room for testing",
        "preset": "public_chat",
    });
    let (status, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &body,
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK, "createRoom failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let room_id = json["room_id"].as_str().unwrap();

    // Check state was set
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/state/m.room.name"),
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK, "get name state failed: {resp}");
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["name"], "Test Room");
}

#[tokio::test]
async fn test_create_room_unauthenticated() {
    let router = common::test_router();
    let body = serde_json::json!({});
    let (status, _) = common::post_json(
        &router,
        "/_matrix/client/v3/createRoom",
        &body,
    ).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_joined_rooms() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "joiner", "pass").await;

    // Create two rooms
    let body = serde_json::json!({});
    common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;
    common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;

    let (status, resp) = common::get_authed(
        &router,
        "/_matrix/client/v3/joined_rooms",
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let rooms = json["joined_rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 2);
}

#[tokio::test]
async fn test_leave_room() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "leaver", "pass").await;

    // Create room
    let (_, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &serde_json::json!({}),
        &token,
    ).await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str().unwrap().to_string();

    // Leave
    let (status, _) = common::post_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/leave"),
        &serde_json::json!({}),
        &token,
    ).await;
    assert_eq!(status, StatusCode::OK);

    // Should not be in joined_rooms anymore
    let (_, resp) = common::get_authed(&router, "/_matrix/client/v3/joined_rooms", &token).await;
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let rooms = json["joined_rooms"].as_array().unwrap();
    assert!(rooms.is_empty());
}

#[tokio::test]
async fn test_invite_and_join() {
    let router = common::test_router();
    let (token_alice, _, _) = common::register_user(&router, "alice2", "pass").await;
    let (token_bob, _, _) = common::register_user(&router, "bob2", "pass").await;

    // Alice creates a room
    let (_, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &serde_json::json!({"preset": "private_chat"}),
        &token_alice,
    ).await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str().unwrap().to_string();

    // Alice invites Bob
    let (status, resp) = common::post_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/invite"),
        &serde_json::json!({"user_id": "@bob2:localhost"}),
        &token_alice,
    ).await;
    assert_eq!(status, StatusCode::OK, "invite failed: {resp}");

    // Bob joins
    let (status, resp) = common::post_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/join"),
        &serde_json::json!({}),
        &token_bob,
    ).await;
    assert_eq!(status, StatusCode::OK, "join failed: {resp}");

    // Bob should see the room in joined_rooms
    let (_, resp) = common::get_authed(&router, "/_matrix/client/v3/joined_rooms", &token_bob).await;
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let rooms = json["joined_rooms"].as_array().unwrap();
    assert!(rooms.iter().any(|r| r.as_str() == Some(&room_id)));
}
