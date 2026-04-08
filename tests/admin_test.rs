use axum::body::Body;
use http::{Request, StatusCode};
use maelstrom_core::identifiers::ServerName;
use maelstrom_storage::mock::MockStorage;
use maelstrom_storage::traits::{DeviceRecord, DeviceStore, UserRecord, UserStore};
use tower::ServiceExt;

fn admin_router() -> axum::Router {
    let storage = MockStorage::new();
    let state = maelstrom_admin::AdminState::new(storage, ServerName::new("localhost"));
    maelstrom_admin::router::build(state)
}

async fn setup_admin_user(storage: &MockStorage) -> String {
    // Create an admin user
    let user = UserRecord {
        localpart: "admin".to_string(),
        password_hash: Some("hash".to_string()),
        is_admin: true,
        is_guest: false,
        is_deactivated: false,
        created_at: chrono::Utc::now(),
    };
    storage.create_user(&user).await.unwrap();

    // Create a device with a token
    let device = DeviceRecord {
        device_id: "ADMINDEV".to_string(),
        user_id: "@admin:localhost".to_string(),
        display_name: None,
        access_token: "admin_token_123".to_string(),
        created_at: chrono::Utc::now(),
    };
    storage.create_device(&device).await.unwrap();

    "admin_token_123".to_string()
}

#[tokio::test]
async fn test_admin_api_requires_auth() {
    let router = admin_router();

    let req = Request::builder()
        .uri("/_maelstrom/admin/v1/server/info")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_admin_api_rejects_non_admin() {
    let storage = MockStorage::new();

    // Create a non-admin user
    let user = UserRecord {
        localpart: "regular".to_string(),
        password_hash: Some("hash".to_string()),
        is_admin: false,
        is_guest: false,
        is_deactivated: false,
        created_at: chrono::Utc::now(),
    };
    storage.create_user(&user).await.unwrap();

    let device = DeviceRecord {
        device_id: "DEV1".to_string(),
        user_id: "@regular:localhost".to_string(),
        display_name: None,
        access_token: "regular_token".to_string(),
        created_at: chrono::Utc::now(),
    };
    storage.create_device(&device).await.unwrap();

    let state = maelstrom_admin::AdminState::new(storage, ServerName::new("localhost"));
    let router = maelstrom_admin::router::build(state);

    let req = Request::builder()
        .uri("/_maelstrom/admin/v1/server/info")
        .method("GET")
        .header("Authorization", "Bearer regular_token")
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_admin_server_info() {
    let storage = MockStorage::new();
    let token = setup_admin_user(&storage).await;

    let state = maelstrom_admin::AdminState::new(storage, ServerName::new("localhost"));
    let router = maelstrom_admin::router::build(state);

    let req = Request::builder()
        .uri("/_maelstrom/admin/v1/server/info")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["server_name"], "localhost");
    assert!(json["version"].is_string());
    assert!(json["uptime_seconds"].is_number());
    assert!(json["system"]["total_memory_mb"].is_number());
}

#[tokio::test]
async fn test_admin_prometheus_metrics() {
    let storage = MockStorage::new();
    let token = setup_admin_user(&storage).await;

    let state = maelstrom_admin::AdminState::new(storage, ServerName::new("localhost"));
    let router = maelstrom_admin::router::build(state);

    let req = Request::builder()
        .uri("/_maelstrom/admin/v1/metrics")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("maelstrom_uptime_seconds"));
    assert!(text.contains("maelstrom_database_up"));
    assert!(text.contains("maelstrom_memory_used_bytes"));
}

#[tokio::test]
async fn test_admin_get_user() {
    let storage = MockStorage::new();
    let token = setup_admin_user(&storage).await;

    let state = maelstrom_admin::AdminState::new(storage, ServerName::new("localhost"));
    let router = maelstrom_admin::router::build(state);

    let req = Request::builder()
        .uri("/_maelstrom/admin/v1/users/@admin:localhost")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["localpart"], "admin");
    assert_eq!(json["is_admin"], true);
}

#[tokio::test]
async fn test_admin_dashboard_page() {
    let storage = MockStorage::new();
    let token = setup_admin_user(&storage).await;

    let state = maelstrom_admin::AdminState::new(storage, ServerName::new("localhost"));
    let router = maelstrom_admin::router::build(state);

    let req = Request::builder()
        .uri("/_maelstrom/admin/")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();

    // Verify semantic HTML structure
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("<main>"));
    assert!(html.contains("<h1>Server Overview</h1>"));
    assert!(html.contains("localhost")); // server name
    assert!(html.contains("<nav")); // navigation
    assert!(html.contains("<dl")); // definition list for status
    // No inline styles
    assert!(!html.contains("style="));
}
