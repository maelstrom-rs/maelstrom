mod common;

use http::StatusCode;

#[tokio::test]
async fn test_liveness_returns_ok() {
    let router = common::test_router();
    let (status, body) = common::get(&router, "/_health/live").await;

    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_readiness_returns_ok_when_healthy() {
    let router = common::test_router();
    let (status, body) = common::get(&router, "/_health/ready").await;

    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "ok");
}
