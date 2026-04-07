mod common;

use http::StatusCode;

#[tokio::test]
async fn test_versions_returns_supported_versions() {
    let router = common::test_router();
    let (status, body) = common::get(&router, "/_matrix/client/versions").await;

    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let versions = json["versions"].as_array().unwrap();

    assert!(!versions.is_empty(), "versions list should not be empty");
    assert!(
        versions.iter().any(|v| v.as_str() == Some("v1.12")),
        "should include v1.12"
    );
}

#[tokio::test]
async fn test_versions_content_type_is_json() {
    let router = common::test_router();

    let req = http::Request::builder()
        .uri("/_matrix/client/versions")
        .method("GET")
        .body(axum::body::Body::empty())
        .unwrap();

    let response = tower::ServiceExt::oneshot(router, req).await.unwrap();

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v: &http::HeaderValue| v.to_str().ok())
        .unwrap_or("");

    assert!(
        content_type.contains("application/json"),
        "Content-Type should be application/json, got: {content_type}"
    );
}
