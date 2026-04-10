mod common;

use http::StatusCode;
use maelstrom_core::matrix::{keys::KeyPair, signing};

// -- Canonical JSON tests --

#[test]
fn test_canonical_json_spec_compliance() {
    // Matrix spec example
    let val = serde_json::json!({
        "auth": {"success": true, "mxid": "@john.doe:example.com", "profile": {"display_name": "John Doe", "three_pids": [{"medium": "email", "address": "john.doe@example.org"}, {"medium": "msisdn", "address": "123456789"}]}},
        "one": 1,
        "two": "Two"
    });

    let canonical = signing::canonical_json(&val);

    // Keys must be sorted at all levels
    assert!(canonical.starts_with(r#"{"auth":{""#));
    assert!(canonical.contains(r#""one":1"#));
    assert!(canonical.contains(r#""two":"Two""#));
    // No structural whitespace (spaces inside string values are fine)
    assert!(!canonical.contains(": "));
    assert!(!canonical.contains(", "));
}

// -- Event signing tests --

#[test]
fn test_sign_event_roundtrip() {
    let kp = KeyPair::generate();

    let event = serde_json::json!({
        "room_id": "!test:example.com",
        "sender": "@alice:example.com",
        "type": "m.room.message",
        "content": {"body": "Hello federation!", "msgtype": "m.text"},
        "origin_server_ts": 1234567890,
        "depth": 1,
    });

    let signed = signing::sign_event(&event, &kp, "example.com");

    // Must have signatures and hashes
    assert!(signed.get("signatures").is_some());
    assert!(signed.get("hashes").is_some());
    assert!(signed["hashes"]["sha256"].is_string());

    // Must verify
    assert!(signing::verify_event_signature(
        &signed,
        &kp.public_key_bytes(),
        "example.com",
        kp.key_id(),
    ));
}

#[test]
fn test_reference_hash_deterministic() {
    let event = serde_json::json!({
        "room_id": "!test:example.com",
        "type": "m.room.message",
        "content": {"body": "deterministic"},
    });

    let hash1 = signing::reference_hash(&event);
    let hash2 = signing::reference_hash(&event);

    assert_eq!(hash1, hash2);
    assert!(hash1.starts_with('$'));
}

// -- HTTP request signing tests --

#[test]
fn test_request_signing_roundtrip() {
    let kp = KeyPair::generate();

    let auth = maelstrom_federation::signing::sign_request(
        &kp,
        "alice.example.com",
        "bob.example.com",
        "PUT",
        "/_matrix/federation/v1/send/txn123",
        Some(&serde_json::json!({"pdus": [], "edus": []})),
    );

    assert!(auth.starts_with("X-Matrix "));
    assert!(auth.contains("origin=\"alice.example.com\""));
    assert!(auth.contains("destination=\"bob.example.com\""));

    let (origin, key_id, sig) =
        maelstrom_federation::signing::parse_x_matrix_header(&auth).unwrap();

    assert_eq!(origin, "alice.example.com");
    assert_eq!(key_id, kp.key_id());

    assert!(maelstrom_federation::signing::verify_request(
        &kp.public_key_bytes(),
        "alice.example.com",
        "bob.example.com",
        "PUT",
        "/_matrix/federation/v1/send/txn123",
        Some(&serde_json::json!({"pdus": [], "edus": []})),
        &sig,
    ));
}

// -- Key server endpoint tests --

#[tokio::test]
async fn test_key_server_endpoint() {
    use axum::body::Body;
    use maelstrom_core::matrix::ephemeral::EphemeralStore;
    use maelstrom_core::matrix::id::ServerName;
    use maelstrom_storage::mock::MockStorage;
    use std::sync::Arc;
    use tower::ServiceExt;

    let storage = MockStorage::new();
    let signing_key = KeyPair::generate();
    let server_name = ServerName::new("localhost");

    let fed_state = maelstrom_federation::FederationState::new(
        storage,
        Arc::new(EphemeralStore::new()),
        signing_key.clone(),
        server_name,
    );

    let router = maelstrom_federation::router::build(fed_state);

    let req = http::Request::builder()
        .uri("/_matrix/key/v2/server")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["server_name"], "localhost");
    assert!(json["verify_keys"][signing_key.key_id()]["key"].is_string());
    assert!(json["signatures"]["localhost"][signing_key.key_id()].is_string());
    assert!(json["valid_until_ts"].is_number());
}

// -- Federation storage tests --

#[tokio::test]
async fn test_mock_federation_key_store() {
    use chrono::Utc;
    use maelstrom_storage::mock::MockStorage;
    use maelstrom_storage::traits::{FederationKeyStore, RemoteKeyRecord, ServerKeyRecord};

    let store = MockStorage::new();

    // Store a server signing key
    let key = ServerKeyRecord {
        key_id: "ed25519:test123".to_string(),
        algorithm: "ed25519".to_string(),
        public_key: "base64pubkey".to_string(),
        private_key: "base64privkey".to_string(),
        valid_until: Utc::now() + chrono::Duration::days(30),
    };

    store.store_server_key(&key).await.unwrap();

    // Retrieve it
    let fetched = store.get_server_key("ed25519:test123").await.unwrap();
    assert_eq!(fetched.public_key, "base64pubkey");

    // List active keys
    let keys = store.get_active_server_keys().await.unwrap();
    assert_eq!(keys.len(), 1);

    // Store remote server keys
    let remote_key = RemoteKeyRecord {
        server_name: "remote.example.com".to_string(),
        key_id: "ed25519:remote1".to_string(),
        public_key: "remotepubkey".to_string(),
        valid_until: Utc::now() + chrono::Duration::days(7),
    };

    store.store_remote_server_keys(&[remote_key]).await.unwrap();

    let remote_keys = store
        .get_remote_server_keys("remote.example.com")
        .await
        .unwrap();
    assert_eq!(remote_keys.len(), 1);
    assert_eq!(remote_keys[0].key_id, "ed25519:remote1");

    // Federation transaction dedup
    store
        .store_federation_txn("remote.example.com", "txn1")
        .await
        .unwrap();
    assert!(
        store
            .has_federation_txn("remote.example.com", "txn1")
            .await
            .unwrap()
    );
    assert!(
        !store
            .has_federation_txn("remote.example.com", "txn2")
            .await
            .unwrap()
    );
}

// -- Pdu federation fields test --

#[test]
fn test_pdu_federation_fields() {
    let event = maelstrom_core::matrix::event::Pdu {
        event_id: "$test123".to_string(),
        room_id: "!room:example.com".to_string(),
        sender: "@alice:example.com".to_string(),
        event_type: "m.room.message".to_string(),
        state_key: None,
        content: serde_json::json!({"body": "hi"}),
        origin_server_ts: 1234567890,
        unsigned: None,
        stream_position: 1,
        origin: Some("example.com".to_string()),
        auth_events: Some(vec!["$auth1".to_string()]),
        prev_events: Some(vec!["$prev1".to_string()]),
        depth: Some(5),
        hashes: Some(serde_json::json!({"sha256": "abc"})),
        signatures: Some(serde_json::json!({"example.com": {"ed25519:key1": "sig"}})),
    };

    let fed_event = event.to_federation_json();
    assert_eq!(fed_event["origin"], "example.com");
    assert_eq!(fed_event["auth_events"][0], "$auth1");
    assert_eq!(fed_event["prev_events"][0], "$prev1");
    assert_eq!(fed_event["depth"], 5);
    assert!(fed_event["hashes"]["sha256"].is_string());
    assert!(fed_event["signatures"]["example.com"]["ed25519:key1"].is_string());

    // Client event should NOT include federation fields
    let client_event = event.to_client_event().into_json();
    assert!(client_event.get("origin").is_none());
    assert!(client_event.get("auth_events").is_none());
    assert!(client_event.get("signatures").is_none());
}
