//! Matrix event signing and verification — the full cryptographic pipeline.
//!
//! # How event signing works
//!
//! When a homeserver creates an event, it must prove authorship and protect the
//! event's integrity. The signing pipeline has these steps:
//!
//! 1. **Strip transient fields** — remove `signatures`, `unsigned`, and
//!    `hashes` from the event. These fields are either computed (signatures,
//!    hashes) or carry non-authoritative metadata (unsigned), so they must not
//!    be included in what gets signed.
//!
//! 2. **Canonical JSON** — serialize the stripped event into canonical JSON
//!    (sorted keys, no floats, no whitespace). This guarantees that every
//!    server produces the same bytes for the same logical event.
//!
//! 3. **Content hash** — SHA-256 the canonical JSON (with `signatures`,
//!    `unsigned`, AND `hashes` stripped). This hash is stored in
//!    `hashes.sha256` and serves as an integrity check: if any content field
//!    was modified after signing, the hash will not match. Servers receiving an
//!    event over federation can recompute this hash to detect tampering.
//!
//! 4. **Ed25519 sign** — sign the canonical JSON (with `signatures` and
//!    `unsigned` stripped, but `hashes` INCLUDED — so the signature covers the
//!    content hash too). The signature is stored in
//!    `signatures.<server_name>.<key_id>`.
//!
//! 5. **Reference hash** — for room versions v4+, the event ID is derived
//!    from the event itself rather than being randomly assigned. The reference
//!    hash is SHA-256 of the canonical JSON (with `signatures` and `unsigned`
//!    stripped), URL-safe base64 encoded, prefixed with `$`. This gives every
//!    server the same event ID for the same event without coordination.
//!
//! # Verification
//!
//! To verify an event received from another server:
//! 1. Look up the signing server's public key (from `/_matrix/key/v2/server`)
//! 2. Strip `signatures` and `unsigned` from the event
//! 3. Serialize to canonical JSON
//! 4. Verify the Ed25519 signature against the canonical bytes
//!
//! # API levels
//!
//! This module provides two API levels:
//! - **`*_canonical` functions** — operate on [`CanonicalJson`] directly.
//!   Preferred for code that already has canonical form.
//! - **Convenience wrappers** — accept `serde_json::Value` and convert
//!   automatically. Panic if the value contains floats.
//!
//! Event-specific operations (which fields to strip) also live on
//! `Pdu` methods — `pdu.sign()`, `pdu.content_hash()`, `pdu.reference_hash()`.

use sha2::{Digest, Sha256};

use super::json::CanonicalJson;
use super::keys::KeyPair;

pub use super::keys::{self, verify_signature};

// ── Core operations on CanonicalJson ───────────────────────────────

/// Clone a canonical event and remove the given top-level keys.
///
/// Different stages of the signing pipeline strip different fields:
/// - **Content hash**: strips `signatures`, `unsigned`, and `hashes` — so the
///   hash covers only the "real" content of the event.
/// - **Signing and reference hash**: strips `signatures` and `unsigned` — so
///   the signature covers the content hash (in `hashes`) but not previous
///   signatures or non-authoritative metadata.
fn strip_fields(event: &CanonicalJson, fields: &[&str]) -> CanonicalJson {
    let mut stripped = event.clone();
    for field in fields {
        stripped.remove(field);
    }
    stripped
}

/// Compute the SHA-256 content hash of a canonical event.
///
/// The content hash is an integrity check. After stripping `signatures`,
/// `unsigned`, and `hashes`, the remaining fields are serialized to canonical
/// JSON and SHA-256 hashed. The result (unpadded base64) is stored in
/// `hashes.sha256` on the signed event.
///
/// When a server receives an event over federation, it recomputes this hash
/// and compares it to `hashes.sha256`. If they differ, the event content was
/// modified after signing (e.g., a field was redacted or tampered with).
pub fn content_hash_canonical(event: &CanonicalJson) -> String {
    let stripped = strip_fields(event, &["signatures", "unsigned", "hashes"]);
    hash_sha256_b64(stripped.encode().as_bytes())
}

/// Compute the reference hash, which becomes the event ID in room versions v4+.
///
/// In older room versions, event IDs were assigned by the creating server
/// (e.g., `$random:server.name`). In v4+, the event ID is derived
/// deterministically from the event content itself: strip `signatures` and
/// `unsigned`, serialize to canonical JSON, SHA-256 hash, and URL-safe base64
/// encode with a `$` prefix.
///
/// This means every server independently computes the same event ID for the
/// same event, without needing to trust the originating server's ID assignment.
/// The `hashes` field is intentionally kept (not stripped), so the reference
/// hash covers the content hash too.
pub fn reference_hash_canonical(event: &CanonicalJson) -> String {
    let stripped = strip_fields(event, &["signatures", "unsigned"]);

    let hash = Sha256::digest(stripped.encode().as_bytes());

    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!("${}", engine.encode(hash))
}

/// Sign a canonical JSON event, adding `hashes` and `signatures` fields.
///
/// This is the main signing entry point. It performs the full pipeline:
///
/// 1. Computes the content hash (SHA-256 of the event without `signatures`,
///    `unsigned`, or `hashes`) and inserts it as `hashes.sha256`.
/// 2. Strips `signatures` and `unsigned` from the event (but keeps `hashes`,
///    so the signature covers the content hash).
/// 3. Serializes to canonical JSON and Ed25519 signs the bytes.
/// 4. Inserts the signature as `signatures.<server_name>.<key_id>`.
///
/// The returned event has both `hashes` and `signatures` populated and is
/// ready to be persisted or sent over federation.
pub fn sign_event_canonical(
    event: &CanonicalJson,
    key: &KeyPair,
    server_name: &str,
) -> CanonicalJson {
    let mut signed = event.clone();

    // Compute and set content hash
    let hash = content_hash_canonical(event);
    let hashes =
        CanonicalJson::from_value(&serde_json::json!({ "sha256": hash })).expect("no floats");
    if let Some(obj) = signed.as_object_mut() {
        obj.insert("hashes".to_owned(), hashes);
    }

    // Build the bytes to sign: strip signatures and unsigned
    let to_sign = strip_fields(&signed, &["signatures", "unsigned"]);
    let canonical_bytes = to_sign.encode();
    let signature = key.sign(canonical_bytes.as_bytes());

    // Add signatures
    let sig_obj = CanonicalJson::from_value(&serde_json::json!({
        server_name: {
            key.key_id(): signature
        }
    }))
    .expect("no floats");
    if let Some(obj) = signed.as_object_mut() {
        obj.insert("signatures".to_owned(), sig_obj);
    }

    signed
}

/// Verify a canonical event's Ed25519 signature from a specific server and key.
///
/// Performs the reverse of signing:
/// 1. Extracts the base64 signature from `signatures.<server_name>.<key_id>`.
/// 2. Strips `signatures` and `unsigned` from the event.
/// 3. Serializes to canonical JSON.
/// 4. Verifies the Ed25519 signature against the canonical bytes using the
///    provided public key.
///
/// Returns `false` if the signature is missing, malformed, or invalid.
pub fn verify_event_signature_canonical(
    event: &CanonicalJson,
    public_key_bytes: &[u8; 32],
    server_name: &str,
    key_id: &str,
) -> bool {
    // Extract signature
    let sig_b64 = event
        .as_object()
        .and_then(|obj| obj.get("signatures"))
        .and_then(|s| s.as_object())
        .and_then(|s| s.get(server_name))
        .and_then(|s| s.as_object())
        .and_then(|s| s.get(key_id))
        .and_then(|v| match v {
            CanonicalJson::String(s) => Some(s.as_str()),
            _ => None,
        });

    let sig_b64 = match sig_b64 {
        Some(s) => s,
        None => return false,
    };

    let to_verify = strip_fields(event, &["signatures", "unsigned"]);
    let canonical_bytes = to_verify.encode();
    verify_signature(public_key_bytes, canonical_bytes.as_bytes(), sig_b64)
}

// ── Convenience wrappers for serde_json::Value ──────────────────────────

/// Convenience wrapper: compute the content hash from a `serde_json::Value`.
///
/// Converts to [`CanonicalJson`] first, then delegates to
/// [`content_hash_canonical`]. Panics if the value contains floats.
pub fn content_hash(event: &serde_json::Value) -> String {
    let canonical = CanonicalJson::from_value(event)
        .expect("Event contains float — not valid for canonical JSON");
    content_hash_canonical(&canonical)
}

/// Convenience wrapper: compute the reference hash (event ID) from a `serde_json::Value`.
///
/// Converts to [`CanonicalJson`] first, then delegates to
/// [`reference_hash_canonical`]. Panics if the value contains floats.
pub fn reference_hash(event: &serde_json::Value) -> String {
    let canonical = CanonicalJson::from_value(event)
        .expect("Event contains float — not valid for canonical JSON");
    reference_hash_canonical(&canonical)
}

/// Convenience wrapper: sign an event from a `serde_json::Value`.
///
/// Converts to [`CanonicalJson`], signs via [`sign_event_canonical`], and
/// converts the result back to `serde_json::Value`. Panics if the value
/// contains floats.
pub fn sign_event(
    event: &serde_json::Value,
    key: &KeyPair,
    server_name: &str,
) -> serde_json::Value {
    let canonical = CanonicalJson::from_value(event)
        .expect("Event contains float — not valid for canonical JSON");
    sign_event_canonical(&canonical, key, server_name).into_value()
}

/// Convenience wrapper: verify an event signature from a `serde_json::Value`.
///
/// Converts to [`CanonicalJson`] and delegates to
/// [`verify_event_signature_canonical`]. Returns `false` if the value
/// contains floats (cannot be canonicalized).
pub fn verify_event_signature(
    event: &serde_json::Value,
    public_key_bytes: &[u8; 32],
    server_name: &str,
    key_id: &str,
) -> bool {
    let Ok(canonical) = CanonicalJson::from_value(event) else {
        return false;
    };
    verify_event_signature_canonical(&canonical, public_key_bytes, server_name, key_id)
}

/// Convenience wrapper: produce the canonical JSON string of a `serde_json::Value`.
///
/// Prefer [`CanonicalJson::from_value()`] + [`encode()`](CanonicalJson::encode)
/// for new code — this function exists for backward compatibility. Panics if
/// the value contains floats.
pub fn canonical_json(value: &serde_json::Value) -> String {
    let canonical = CanonicalJson::from_value(value)
        .expect("Value contains float — not valid for canonical JSON");
    canonical.encode()
}

// ── Internal ────────────────────────────────────────────────────────────

/// SHA-256 hash, returned as unpadded standard base64.
///
/// Internal helper used by [`content_hash_canonical`]. The base64 encoding
/// uses the standard alphabet (not URL-safe) without padding, as required
/// by the Matrix spec for content hashes.
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
        let kp = KeyPair::generate();
        let event = serde_json::json!({
            "room_id": "!test:example.com",
            "sender": "@alice:example.com",
            "type": "m.room.message",
            "content": {"body": "hello", "msgtype": "m.text"},
            "origin_server_ts": 1234567890,
        });

        let signed = sign_event(&event, &kp, "example.com");

        assert!(verify_event_signature(
            &signed,
            &kp.public_key_bytes(),
            "example.com",
            kp.key_id(),
        ));

        assert!(signed["hashes"]["sha256"].is_string());
        assert!(signed["signatures"]["example.com"][kp.key_id()].is_string());
    }

    #[test]
    fn test_sign_and_verify_canonical_api() {
        let kp = KeyPair::generate();
        let event = CanonicalJson::from_value(&serde_json::json!({
            "room_id": "!test:example.com",
            "sender": "@alice:example.com",
            "type": "m.room.message",
            "content": {"body": "hello"},
            "origin_server_ts": 1234567890,
        }))
        .unwrap();

        let signed = sign_event_canonical(&event, &kp, "example.com");

        assert!(verify_event_signature_canonical(
            &signed,
            &kp.public_key_bytes(),
            "example.com",
            kp.key_id(),
        ));
    }

    #[test]
    fn test_tampered_event_fails_verify() {
        let kp = KeyPair::generate();
        let event = serde_json::json!({
            "content": {"body": "original"},
            "type": "m.room.message",
        });

        let mut signed = sign_event(&event, &kp, "example.com");
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
        assert!(ref_hash.len() > 40);
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

        assert_eq!(content_hash(&event1), content_hash(&event2));
    }

    #[test]
    fn test_float_rejected_in_sign() {
        let result = std::panic::catch_unwind(|| {
            let event = serde_json::json!({"value": 1.5});
            canonical_json(&event);
        });
        assert!(result.is_err());
    }
}
