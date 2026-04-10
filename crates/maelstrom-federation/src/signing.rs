//! # X-Matrix Request Signing
//!
//! Every federation HTTP request in Matrix is authenticated using the **X-Matrix**
//! authorization scheme. This module handles both signing outbound requests and
//! verifying inbound ones.
//!
//! ## What Gets Signed
//!
//! The signature is computed over a JSON object containing:
//!
//! - `method` -- the HTTP method (e.g., `GET`, `PUT`)
//! - `uri` -- the request path (e.g., `/_matrix/federation/v1/send/txn123`)
//! - `origin` -- the sending server's name (e.g., `alice.com`)
//! - `destination` -- the receiving server's name (e.g., `bob.org`)
//! - `content` -- the request body (only for requests that have one, like PUT)
//!
//! This JSON object is converted to **canonical JSON** (deterministic key ordering,
//! no optional whitespace) and then signed with the server's Ed25519 key.
//!
//! ## Authorization Header Format
//!
//! The resulting `Authorization` header looks like:
//!
//! ```text
//! X-Matrix origin="alice.com",destination="bob.org",key="ed25519:abc123",sig="<base64>"
//! ```
//!
//! The receiving server parses this header, looks up the origin server's public key
//! (from `/_matrix/key/v2/server` or a cached copy), and verifies the signature
//! against the same canonical JSON reconstruction of the request.
//!
//! ## Functions
//!
//! - [`sign_request`] -- produce an `Authorization` header value for an outbound request
//! - [`parse_x_matrix_header`] -- extract `(origin, key_id, signature)` from an inbound header
//! - [`verify_request`] -- verify an inbound request signature against a known public key

use maelstrom_core::matrix::json::CanonicalJson;
use maelstrom_core::matrix::keys::KeyPair;

/// Sign an outbound federation HTTP request.
///
/// Constructs the canonical JSON object from the request parameters, signs it with
/// the provided key pair, and returns the full `Authorization` header value in the
/// `X-Matrix` scheme.
///
/// If the request has a body (e.g., PUT requests), pass it as `content` so it is
/// included in the signature. GET requests should pass `None`.
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

    let canonical =
        CanonicalJson::from_value(&obj).expect("Request object should not contain floats");
    let signature = key.sign(canonical.encode().as_bytes());

    format!(
        "X-Matrix origin=\"{origin}\",destination=\"{destination}\",key=\"{}\",sig=\"{signature}\"",
        key.key_id()
    )
}

/// Parse an inbound `Authorization: X-Matrix` header.
///
/// Extracts the three key fields from the header value:
///
/// - **origin** -- the server that sent the request (e.g., `alice.com`)
/// - **key_id** -- which signing key was used (e.g., `ed25519:abc123`)
/// - **signature** -- the base64-encoded Ed25519 signature
///
/// Returns `None` if the header is missing the `X-Matrix` prefix or any of the
/// three required fields.
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
///
/// Reconstructs the same canonical JSON that the sending server signed (from the
/// method, URI, origin, destination, and optional body), then verifies the provided
/// base64 signature against the sender's public key.
///
/// Returns `true` if the signature is valid, `false` otherwise (including if the
/// canonical JSON cannot be constructed or the signature is malformed).
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

    let Ok(canonical) = CanonicalJson::from_value(&obj) else {
        return false;
    };
    maelstrom_core::matrix::keys::verify_signature(
        public_key_bytes,
        canonical.encode().as_bytes(),
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
