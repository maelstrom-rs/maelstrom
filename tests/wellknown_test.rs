mod common;

use http::StatusCode;

#[tokio::test]
async fn test_wellknown_returns_homeserver_info() {
    let router = common::test_router();
    let (status, body) = common::get(&router, "/.well-known/matrix/client").await;

    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let base_url = json["m.homeserver"]["base_url"].as_str().unwrap();

    assert_eq!(base_url, "http://localhost:8008");
}
