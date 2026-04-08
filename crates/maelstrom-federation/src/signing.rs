use maelstrom_core::signatures::{canonical_json, keys::KeyPair};

/// Sign an outbound federation HTTP request.
///
/// Returns the `Authorization` header value in the `X-Matrix` scheme.
pub fn sign_request(
    key: &KeyPair,
    origin: &str,
    destination: &str,
    method: &str,
    uri: &str,
    content: Option<&serde_json::Value>,
) -> String {
    let mut obj = serde_json::json!({
        "method": method,
        "uri": uri,
        "origin": origin,
        "destination": destination,
    });

    if let Some(body) = content {
        obj["content"] = body.clone();
    }

    let canonical = canonical_json(&obj);
    let signature = key.sign(canonical.as_bytes());

    format!(
        "X-Matrix origin=\"{origin}\",destination=\"{destination}\",key=\"{}\",sig=\"{signature}\"",
        key.key_id()
    )
}

/// Parse and verify an inbound `Authorization: X-Matrix` header.
///
/// Returns `(origin, key_id, signature)` if the header is well-formed.
pub fn parse_x_matrix_header(header: &str) -> Option<(String, String, String)> {
    let header = header.strip_prefix("X-Matrix ")?;

    let mut origin = None;
    let mut key_id = None;
    let mut sig = None;

    for part in header.split(',') {
        let part = part.trim();
        if let Some(val) = part
            .strip_prefix("origin=\"")
            .and_then(|s| s.strip_suffix('"'))
        {
            origin = Some(val.to_string());
        } else if let Some(val) = part
            .strip_prefix("key=\"")
            .and_then(|s| s.strip_suffix('"'))
        {
            key_id = Some(val.to_string());
        } else if let Some(val) = part
            .strip_prefix("sig=\"")
            .and_then(|s| s.strip_suffix('"'))
        {
            sig = Some(val.to_string());
        }
    }

    Some((origin?, key_id?, sig?))
}

/// Verify an inbound federation request signature.
pub fn verify_request(
    public_key_bytes: &[u8; 32],
    origin: &str,
    destination: &str,
    method: &str,
    uri: &str,
    content: Option<&serde_json::Value>,
    signature_b64: &str,
) -> bool {
    let mut obj = serde_json::json!({
        "method": method,
        "uri": uri,
        "origin": origin,
        "destination": destination,
    });

    if let Some(body) = content {
        obj["content"] = body.clone();
    }

    let canonical = canonical_json(&obj);
    maelstrom_core::signatures::keys::verify_signature(
        public_key_bytes,
        canonical.as_bytes(),
        signature_b64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify_request() {
        let kp = KeyPair::generate();
        let auth = sign_request(&kp, "origin.com", "dest.com", "GET", "/path", None);

        let (origin, key_id, sig) = parse_x_matrix_header(&auth).unwrap();
        assert_eq!(origin, "origin.com");
        assert_eq!(key_id, kp.key_id());

        assert!(verify_request(
            &kp.public_key_bytes(),
            "origin.com",
            "dest.com",
            "GET",
            "/path",
            None,
            &sig,
        ));
    }

    #[test]
    fn test_sign_request_with_body() {
        let kp = KeyPair::generate();
        let body = serde_json::json!({"key": "value"});
        let auth = sign_request(&kp, "a.com", "b.com", "PUT", "/send/txn1", Some(&body));

        let (_, _, sig) = parse_x_matrix_header(&auth).unwrap();
        assert!(verify_request(
            &kp.public_key_bytes(),
            "a.com",
            "b.com",
            "PUT",
            "/send/txn1",
            Some(&body),
            &sig,
        ));
    }

    #[test]
    fn test_parse_x_matrix_header() {
        let header =
            r#"X-Matrix origin="a.com",destination="b.com",key="ed25519:abc",sig="base64sig""#;
        let (origin, key, sig) = parse_x_matrix_header(header).unwrap();
        assert_eq!(origin, "a.com");
        assert_eq!(key, "ed25519:abc");
        assert_eq!(sig, "base64sig");
    }
}
