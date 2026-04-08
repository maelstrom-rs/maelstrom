pub mod keys;

use sha2::{Digest, Sha256};

use keys::KeyPair;

/// Produce the Matrix canonical JSON encoding of a value.
///
/// Rules: objects have sorted keys, no floats (only integers), no whitespace,
/// strings use minimal JSON escaping.
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        serde_json::Value::Number(n) => {
            // Matrix canonical JSON requires integers, no floats
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else {
                // Float — shouldn't occur in Matrix events but handle gracefully
                n.to_string()
            }
        }
        serde_json::Value::String(s) => serde_json::to_string(s).unwrap(),
        serde_json::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        serde_json::Value::Object(obj) => {
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys
                .iter()
                .map(|k| {
                    let key_json = serde_json::to_string(*k).unwrap();
                    let val_json = canonical_json(&obj[*k]);
                    format!("{key_json}:{val_json}")
                })
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
    }
}

/// Compute the SHA-256 content hash of an event.
///
/// Per the spec: remove `signatures`, `unsigned`, and `hashes` fields,
/// then SHA-256 the canonical JSON. Returns unpadded base64.
pub fn content_hash(event: &serde_json::Value) -> String {
    let mut redacted = event.clone();
    if let Some(obj) = redacted.as_object_mut() {
        obj.remove("signatures");
        obj.remove("unsigned");
        obj.remove("hashes");
    }

    let canonical = canonical_json(&redacted);
    hash_sha256_b64(canonical.as_bytes())
}

/// Compute the reference hash for a v4+ event ID.
///
/// Per the spec: remove `signatures` and `unsigned`, then SHA-256 the canonical JSON.
/// Returns `$` + URL-safe unpadded base64.
pub fn reference_hash(event: &serde_json::Value) -> String {
    let mut redacted = event.clone();
    if let Some(obj) = redacted.as_object_mut() {
        obj.remove("signatures");
        obj.remove("unsigned");
    }

    let canonical = canonical_json(&redacted);
    let hash = Sha256::digest(canonical.as_bytes());

    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("${}", engine.encode(hash))
}

/// Sign an event JSON object. Adds `hashes` and `signatures` fields.
///
/// The event should NOT already have `signatures` or `hashes` — they will be overwritten.
pub fn sign_event(
    event: &serde_json::Value,
    key: &KeyPair,
    server_name: &str,
) -> serde_json::Value {
    let mut signed = event.clone();

    // Compute and set content hash
    let hash = content_hash(event);
    signed["hashes"] = serde_json::json!({ "sha256": hash });

    // Remove fields that aren't signed
    let mut to_sign = signed.clone();
    if let Some(obj) = to_sign.as_object_mut() {
        obj.remove("signatures");
        obj.remove("unsigned");
    }

    let canonical = canonical_json(&to_sign);
    let signature = key.sign(canonical.as_bytes());

    // Build or merge signatures object
    let sig_obj = serde_json::json!({
        server_name: {
            key.key_id(): signature
        }
    });
    signed["signatures"] = sig_obj;

    signed
}

/// Verify an event's signature from a specific server and key.
pub fn verify_event_signature(
    event: &serde_json::Value,
    public_key_bytes: &[u8; 32],
    server_name: &str,
    key_id: &str,
) -> bool {
    // Extract the signature
    let sig_b64 = match event
        .get("signatures")
        .and_then(|s| s.get(server_name))
        .and_then(|s| s.get(key_id))
        .and_then(|s| s.as_str())
    {
        Some(s) => s,
        None => return false,
    };

    // Recreate the signed content: remove signatures and unsigned
    let mut to_verify = event.clone();
    if let Some(obj) = to_verify.as_object_mut() {
        obj.remove("signatures");
        obj.remove("unsigned");
    }

    let canonical = canonical_json(&to_verify);
    keys::verify_signature(public_key_bytes, canonical.as_bytes(), sig_b64)
}

/// SHA-256 hash, returned as unpadded standard base64.
fn hash_sha256_b64(data: &[u8]) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
    let hash = Sha256::digest(data);
    engine.encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_json_sorted_keys() {
        let val = serde_json::json!({"b": 2, "a": 1});
        assert_eq!(canonical_json(&val), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn test_canonical_json_nested() {
        let val = serde_json::json!({"z": {"b": 1, "a": 2}, "a": []});
        assert_eq!(canonical_json(&val), r#"{"a":[],"z":{"a":2,"b":1}}"#);
    }

    #[test]
    fn test_canonical_json_strings() {
        let val = serde_json::json!({"key": "hello\nworld"});
        assert_eq!(canonical_json(&val), r#"{"key":"hello\nworld"}"#);
    }

    #[test]
    fn test_canonical_json_array() {
        let val = serde_json::json!([3, 1, 2]);
        assert_eq!(canonical_json(&val), "[3,1,2]");
    }

    #[test]
    fn test_sign_and_verify_event() {
        let kp = keys::KeyPair::generate();
        let event = serde_json::json!({
            "room_id": "!test:example.com",
            "sender": "@alice:example.com",
            "type": "m.room.message",
            "content": {"body": "hello", "msgtype": "m.text"},
            "origin_server_ts": 1234567890,
        });

        let signed = sign_event(&event, &kp, "example.com");

        // Verify the signature
        assert!(verify_event_signature(
            &signed,
            &kp.public_key_bytes(),
            "example.com",
            kp.key_id(),
        ));

        // Verify hashes exist
        assert!(signed["hashes"]["sha256"].is_string());
        assert!(signed["signatures"]["example.com"][kp.key_id()].is_string());
    }

    #[test]
    fn test_tampered_event_fails_verify() {
        let kp = keys::KeyPair::generate();
        let event = serde_json::json!({
            "content": {"body": "original"},
            "type": "m.room.message",
        });

        let mut signed = sign_event(&event, &kp, "example.com");

        // Tamper with the content
        signed["content"]["body"] = serde_json::json!("tampered");

        assert!(!verify_event_signature(
            &signed,
            &kp.public_key_bytes(),
            "example.com",
            kp.key_id(),
        ));
    }

    #[test]
    fn test_reference_hash_starts_with_dollar() {
        let event = serde_json::json!({
            "room_id": "!test:example.com",
            "type": "m.room.message",
            "content": {"body": "hello"},
        });

        let ref_hash = reference_hash(&event);
        assert!(ref_hash.starts_with('$'));
        assert!(ref_hash.len() > 40); // base64 of SHA-256 is 43 chars + $
    }

    #[test]
    fn test_content_hash_excludes_signatures_and_unsigned() {
        let event1 = serde_json::json!({
            "content": {"body": "hello"},
            "type": "m.room.message",
        });

        let event2 = serde_json::json!({
            "content": {"body": "hello"},
            "type": "m.room.message",
            "signatures": {"server": {"key": "sig"}},
            "unsigned": {"age": 100},
            "hashes": {"sha256": "old_hash"},
        });

        // Content hash should be the same regardless of signatures/unsigned/hashes
        assert_eq!(content_hash(&event1), content_hash(&event2));
    }
}
