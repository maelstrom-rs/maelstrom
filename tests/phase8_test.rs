mod common;

use http::StatusCode;

// -- Relations --

#[tokio::test]
async fn test_send_reaction_and_get_relations() {
    let router = common::test_router();
    let (token, _user_id, _) = common::register_user(&router, "reactor", "pass").await;

    // Create a room
    let body = serde_json::json!({"preset": "public_chat"});
    let (_, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send a message
    let msg = serde_json::json!({"body": "Hello!", "msgtype": "m.text"});
    let (status, resp) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn1"),
        &msg,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let event_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["event_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send a reaction to it
    let reaction = serde_json::json!({
        "m.relates_to": {
            "rel_type": "m.annotation",
            "event_id": event_id,
            "key": "👍"
        }
    });
    let (status, _) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.reaction/txn2"),
        &reaction,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Get relations
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v1/rooms/{room_id}/relations/{event_id}"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let chunk = json["chunk"].as_array().unwrap();
    assert_eq!(chunk.len(), 1);

    // Get relations filtered by type
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v1/rooms/{room_id}/relations/{event_id}/m.annotation"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["chunk"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_send_thread_reply() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "threader", "pass").await;

    let body = serde_json::json!({"preset": "public_chat"});
    let (_, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send root message
    let msg = serde_json::json!({"body": "Thread root", "msgtype": "m.text"});
    let (_, resp) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn1"),
        &msg,
        &token,
    )
    .await;
    let root_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["event_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send thread reply
    let reply = serde_json::json!({
        "body": "Thread reply",
        "msgtype": "m.text",
        "m.relates_to": {
            "rel_type": "m.thread",
            "event_id": root_id,
        }
    });
    let (status, _) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn2"),
        &reply,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Get threads
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v1/rooms/{room_id}/threads"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let chunk = json["chunk"].as_array().unwrap();
    assert!(!chunk.is_empty());
}

// -- Knocking --

#[tokio::test]
async fn test_knock_on_room() {
    let router = common::test_router();
    let (token_alice, _, _) = common::register_user(&router, "knockalice", "pass").await;
    let (token_bob, _, _) = common::register_user(&router, "knockbob", "pass").await;

    // Alice creates a room with knock join rule
    let body = serde_json::json!({
        "preset": "private_chat",
        "initial_state": [
            {"type": "m.room.join_rules", "state_key": "", "content": {"join_rule": "knock"}}
        ]
    });
    let (_, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &body,
        &token_alice,
    )
    .await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Bob knocks
    let knock_body = serde_json::json!({"reason": "Let me in!"});
    let (status, resp) = common::post_json_authed(
        &router,
        &format!("/_matrix/client/v3/knock/{room_id}"),
        &knock_body,
        &token_bob,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "Knock failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["room_id"].as_str().unwrap(), room_id);
}

#[tokio::test]
async fn test_knock_rejected_on_public_room() {
    let router = common::test_router();
    let (token_alice, _, _) = common::register_user(&router, "knockalice2", "pass").await;
    let (token_bob, _, _) = common::register_user(&router, "knockbob2", "pass").await;

    // Alice creates a public room (join_rule = "public")
    let body = serde_json::json!({"preset": "public_chat"});
    let (_, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/createRoom",
        &body,
        &token_alice,
    )
    .await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Bob tries to knock — should fail (room is public, not knock)
    let knock_body = serde_json::json!({});
    let (status, _) = common::post_json_authed(
        &router,
        &format!("/_matrix/client/v3/knock/{room_id}"),
        &knock_body,
        &token_bob,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// -- Reporting --

#[tokio::test]
async fn test_report_event() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "reporter", "pass").await;

    let body = serde_json::json!({"preset": "public_chat"});
    let (_, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;
    let room_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send a message
    let msg = serde_json::json!({"body": "bad content", "msgtype": "m.text"});
    let (_, resp) = common::put_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/send/m.room.message/txn1"),
        &msg,
        &token,
    )
    .await;
    let event_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["event_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Report it
    let report = serde_json::json!({"reason": "Inappropriate", "score": -50});
    let (status, _) = common::post_json_authed(
        &router,
        &format!("/_matrix/client/v3/rooms/{room_id}/report/{event_id}"),
        &report,
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

// -- Spaces hierarchy --

#[tokio::test]
async fn test_space_hierarchy() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "spacer", "pass").await;

    // Create a space (room with type m.space)
    let body = serde_json::json!({
        "preset": "public_chat",
        "initial_state": [
            {"type": "m.room.create", "state_key": "", "content": {"type": "m.space"}}
        ]
    });
    let (_, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/createRoom", &body, &token).await;
    let space_id = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["room_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Get hierarchy (should return at least the space itself)
    let (status, resp) = common::get_authed(
        &router,
        &format!("/_matrix/client/v1/rooms/{space_id}/hierarchy"),
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let rooms = json["rooms"].as_array().unwrap();
    assert!(!rooms.is_empty());
}

// -- MockStorage relation tests --

#[tokio::test]
async fn test_mock_relation_store() {
    use maelstrom_storage::mock::MockStorage;
    use maelstrom_storage::traits::{RelationRecord, RelationStore};

    let store = MockStorage::new();

    let relation = RelationRecord {
        event_id: "$reaction1".to_string(),
        parent_id: "$msg1".to_string(),
        room_id: "!room:localhost".to_string(),
        rel_type: "m.annotation".to_string(),
        sender: "@alice:localhost".to_string(),
        event_type: "m.reaction".to_string(),
        content_key: Some("👍".to_string()),
    };

    store.store_relation(&relation).await.unwrap();

    // Get all relations
    let rels = store
        .get_relations("$msg1", None, None, 10, None)
        .await
        .unwrap();
    assert_eq!(rels.len(), 1);

    // Get by type
    let rels = store
        .get_relations("$msg1", Some("m.annotation"), None, 10, None)
        .await
        .unwrap();
    assert_eq!(rels.len(), 1);

    // Get non-matching type
    let rels = store
        .get_relations("$msg1", Some("m.thread"), None, 10, None)
        .await
        .unwrap();
    assert_eq!(rels.len(), 0);

    // Reaction counts
    let counts = store.get_reaction_counts("$msg1").await.unwrap();
    assert_eq!(counts.len(), 1);
    assert_eq!(counts[0], ("👍".to_string(), 1));

    // Latest edit (none)
    let edit = store.get_latest_edit("$msg1").await.unwrap();
    assert!(edit.is_none());

    // Add an edit
    let edit_rel = RelationRecord {
        event_id: "$edit1".to_string(),
        parent_id: "$msg1".to_string(),
        room_id: "!room:localhost".to_string(),
        rel_type: "m.replace".to_string(),
        sender: "@alice:localhost".to_string(),
        event_type: "m.room.message".to_string(),
        content_key: None,
    };
    store.store_relation(&edit_rel).await.unwrap();

    let edit = store.get_latest_edit("$msg1").await.unwrap();
    assert_eq!(edit.as_deref(), Some("$edit1"));
}
