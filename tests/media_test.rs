mod common;

use http::StatusCode;

#[tokio::test]
async fn test_media_config_returns_upload_size() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "mediauser", "pass").await;

    let (status, resp) =
        common::get_authed(&router, "/_matrix/client/v1/media/config", &token).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let size = json["m.upload.size"].as_u64().unwrap();
    assert!(size > 0, "upload size should be positive");
}

#[tokio::test]
async fn test_media_config_legacy_endpoint() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "legacymedia", "pass").await;

    let (status, resp) = common::get_authed(&router, "/_matrix/media/v3/config", &token).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json["m.upload.size"].as_u64().is_some());
}

#[tokio::test]
async fn test_media_config_requires_auth() {
    let router = common::test_router();

    let (status, resp) = common::get(&router, "/_matrix/client/v1/media/config").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(json["errcode"], "M_MISSING_TOKEN");
}

#[tokio::test]
async fn test_upload_fails_without_media_store() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "uploaduser", "pass").await;

    // Upload with no media store configured (MockStorage has no MediaClient)
    let req = http::Request::builder()
        .uri("/_matrix/client/v1/media/upload")
        .method("POST")
        .header("Content-Type", "image/png")
        .header("Authorization", format!("Bearer {token}"))
        .body(axum::body::Body::from(vec![0u8; 100]))
        .unwrap();

    let response = tower::ServiceExt::oneshot(router.clone(), req)
        .await
        .unwrap();
    let status = response.status();
    // Should return 500 because media store is not configured in test state
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_download_not_found() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "dluser", "pass").await;

    // Try to download non-existent media — should fail since no media store
    let (status, _) = common::get_authed(
        &router,
        "/_matrix/client/v1/media/download/localhost/nonexistent",
        &token,
    )
    .await;
    // Either 500 (no media store) or 404 (not found) is acceptable
    assert!(
        status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND,
        "Expected 500 or 404, got {status}"
    );
}

#[tokio::test]
async fn test_preview_url_returns_empty() {
    let router = common::test_router();
    let (token, _, _) = common::register_user(&router, "previewuser", "pass").await;

    let (status, resp) = common::get_authed(
        &router,
        "/_matrix/client/v1/media/preview_url?url=https%3A%2F%2Fexample.com",
        &token,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    // Preview is a stub — should return a JSON object (may be empty)
    assert!(json.is_object());
}

// -- MockStorage media trait tests --

#[tokio::test]
async fn test_mock_storage_media_crud() {
    use chrono::Utc;
    use maelstrom_storage::mock::MockStorage;
    use maelstrom_storage::traits::{MediaRecord, MediaStore};

    let store = MockStorage::new();

    let record = MediaRecord {
        media_id: "abc123".to_string(),
        server_name: "localhost".to_string(),
        user_id: "@alice:localhost".to_string(),
        content_type: "image/png".to_string(),
        content_length: 1024,
        filename: Some("photo.png".to_string()),
        s3_key: "localhost/abc123".to_string(),
        created_at: Utc::now(),
        quarantined: false,
    };

    // Store
    store.store_media(&record).await.unwrap();

    // Get
    let fetched = store.get_media("localhost", "abc123").await.unwrap();
    assert_eq!(fetched.media_id, "abc123");
    assert_eq!(fetched.content_type, "image/png");
    assert_eq!(fetched.filename.as_deref(), Some("photo.png"));

    // Duplicate should fail
    let dup = store.store_media(&record).await;
    assert!(dup.is_err());

    // Quarantine
    store
        .set_media_quarantined("localhost", "abc123", true)
        .await
        .unwrap();
    let quarantined = store.get_media("localhost", "abc123").await.unwrap();
    assert!(quarantined.quarantined);

    // List by user
    let user_media = store.list_user_media("@alice:localhost", 10).await.unwrap();
    assert_eq!(user_media.len(), 1);

    // Delete
    store.delete_media("localhost", "abc123").await.unwrap();
    let gone = store.get_media("localhost", "abc123").await;
    assert!(gone.is_err());
}

#[tokio::test]
async fn test_mock_storage_list_media_before() {
    use chrono::{Duration, Utc};
    use maelstrom_storage::mock::MockStorage;
    use maelstrom_storage::traits::{MediaRecord, MediaStore};

    let store = MockStorage::new();

    let old_record = MediaRecord {
        media_id: "old1".to_string(),
        server_name: "localhost".to_string(),
        user_id: "@bob:localhost".to_string(),
        content_type: "text/plain".to_string(),
        content_length: 100,
        filename: None,
        s3_key: "localhost/old1".to_string(),
        created_at: Utc::now() - Duration::days(30),
        quarantined: false,
    };

    let new_record = MediaRecord {
        media_id: "new1".to_string(),
        server_name: "localhost".to_string(),
        user_id: "@bob:localhost".to_string(),
        content_type: "text/plain".to_string(),
        content_length: 200,
        filename: None,
        s3_key: "localhost/new1".to_string(),
        created_at: Utc::now(),
        quarantined: false,
    };

    store.store_media(&old_record).await.unwrap();
    store.store_media(&new_record).await.unwrap();

    // Only the old one should appear
    let cutoff = Utc::now() - Duration::days(7);
    let old_media = store.list_media_before(cutoff, 100).await.unwrap();
    assert_eq!(old_media.len(), 1);
    assert_eq!(old_media[0].media_id, "old1");
}
