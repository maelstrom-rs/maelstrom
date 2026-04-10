mod common;

use http::StatusCode;

#[tokio::test]
async fn test_upload_device_keys() {
    let router = common::test_router();
    let (token, user_id, device_id) = common::register_user(&router, "keyuser", "pass").await;

    let keys = serde_json::json!({
        "device_keys": {
            "user_id": user_id,
            "device_id": device_id,
            "algorithms": ["m.olm.v1.curve25519-aes-sha2", "m.megolm.v1.aes-sha2"],
            "keys": {
                format!("curve25519:{device_id}"): "base64curve25519key",
                format!("ed25519:{device_id}"): "base64ed25519key",
            },
            "signatures": {
                &user_id: {
                    format!("ed25519:{device_id}"): "base64signature",
                }
            }
        }
    });

    let (status, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/keys/upload", &keys, &token).await;
    assert_eq!(status, StatusCode::OK, "Upload keys failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json.get("one_time_key_counts").is_some());
}

#[tokio::test]
async fn test_upload_one_time_keys() {
    let router = common::test_router();
    let (token, _, _device_id) = common::register_user(&router, "otkuser", "pass").await;

    let keys = serde_json::json!({
        "one_time_keys": {
            format!("curve25519:AAAAAQ"): "base64otk1",
            format!("curve25519:AAAABA"): "base64otk2",
        }
    });

    let (status, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/keys/upload", &keys, &token).await;
    assert_eq!(status, StatusCode::OK);

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let counts = &json["one_time_key_counts"];
    assert!(counts.is_object());
}

#[tokio::test]
async fn test_query_device_keys() {
    let router = common::test_router();
    let (token, user_id, _) = common::register_user(&router, "queryuser", "pass").await;

    // First upload keys
    let keys = serde_json::json!({
        "device_keys": {
            "user_id": user_id,
            "device_id": "TESTDEV",
            "algorithms": ["m.olm.v1.curve25519-aes-sha2"],
            "keys": {},
            "signatures": {}
        }
    });
    common::post_json_authed(&router, "/_matrix/client/v3/keys/upload", &keys, &token).await;

    // Query keys
    let query = serde_json::json!({
        "device_keys": {
            &user_id: []
        }
    });

    let (status, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/keys/query", &query, &token).await;
    assert_eq!(status, StatusCode::OK, "Query keys failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json.get("device_keys").is_some());
}

#[tokio::test]
async fn test_claim_one_time_keys() {
    let router = common::test_router();
    let (token1, _user_id1, _device_id1) = common::register_user(&router, "claimer", "pass").await;
    let (token2, user_id2, device_id2) = common::register_user(&router, "claimee", "pass").await;

    // User2 uploads OTKs
    let keys = serde_json::json!({
        "one_time_keys": {
            "curve25519:AAAAAQ": "base64otk_claim",
        }
    });
    common::post_json_authed(&router, "/_matrix/client/v3/keys/upload", &keys, &token2).await;

    // User1 claims
    let claim = serde_json::json!({
        "one_time_keys": {
            &user_id2: {
                &device_id2: "curve25519"
            }
        }
    });

    let (status, resp) =
        common::post_json_authed(&router, "/_matrix/client/v3/keys/claim", &claim, &token1).await;
    assert_eq!(status, StatusCode::OK, "Claim keys failed: {resp}");

    let json: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(json.get("one_time_keys").is_some());
}

#[tokio::test]
async fn test_upload_cross_signing_keys() {
    let router = common::test_router();
    let (token, user_id, _) = common::register_user(&router, "crosssign", "pass").await;

    let keys = serde_json::json!({
        "auth": {
            "type": "m.login.dummy",
        },
        "master_key": {
            "user_id": user_id,
            "usage": ["master"],
            "keys": {"ed25519:masterkey123": "base64masterkey"},
        },
        "self_signing_key": {
            "user_id": user_id,
            "usage": ["self_signing"],
            "keys": {"ed25519:selfsigning123": "base64selfsigningkey"},
        },
    });

    let (status, resp) = common::post_json_authed(
        &router,
        "/_matrix/client/v3/keys/device_signing/upload",
        &keys,
        &token,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "Upload cross-signing keys failed: {resp}"
    );
}
