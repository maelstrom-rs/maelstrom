//! Ed25519 signing keys for Matrix federation and event integrity.
//!
//! Every Matrix homeserver has at least one Ed25519 signing key. This key is
//! used for two critical operations:
//!
//! 1. **Event signing** — when a server creates an event (message, state
//!    change, etc.), it signs the canonical JSON of that event with its key.
//!    Other servers verify this signature to confirm the event really came from
//!    the claimed origin.
//!
//! 2. **Federation request signing** — every HTTP request between servers
//!    includes an `Authorization` header with an Ed25519 signature over the
//!    request method, URI, and body. This authenticates server-to-server
//!    communication.
//!
//! This module wraps [`ed25519_dalek`] to provide a [`KeyPair`] type that
//! handles key generation, serialization, signing, and verification. The
//! standalone [`verify_signature`] function is used to check signatures from
//! remote servers when you only have their public key.

use ed25519_dalek::{Signer, Verifier};
use rand::Rng;

/// An Ed25519 signing keypair for this homeserver.
///
/// # Key ID format
///
/// Each key has an identifier in the format `ed25519:XXXXXXXX`, where the
/// suffix is a random 8-character alphanumeric string generated at creation
/// time. This ID is included in signatures so that verifiers know which key
/// to use (a server may rotate keys over time, so multiple key IDs can exist).
///
/// # Lifecycle
///
/// - **First startup**: call [`KeyPair::generate()`] to create a new random
///   keypair. Store the private key bytes and key ID in the database.
/// - **Subsequent startups**: call [`KeyPair::from_bytes()`] to reload the
///   keypair from storage. The same key must be used consistently — changing
///   keys without proper rotation will break signature verification for
///   existing events.
/// - **Signing**: call [`KeyPair::sign()`] to produce an Ed25519 signature
///   over arbitrary bytes (typically the canonical JSON of an event or HTTP
///   request). Returns unpadded base64.
/// - **Verification**: use the standalone [`verify_signature()`] function with
///   the remote server's public key bytes.
#[derive(Clone)]
pub struct KeyPair {
    key_id: String,
    signing_key: ed25519_dalek::SigningKey,
}

impl KeyPair {
    /// Generate a new random Ed25519 keypair with a random key ID.
    ///
    /// Call this once on first server startup, then persist the private key
    /// bytes and key ID to the database. The key ID is `ed25519:` followed by
    /// 8 random alphanumeric characters (e.g., `ed25519:a1b2c3d4`).
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rng);

        // Generate a short random key ID
        let id_chars: String = (0..8)
            .map(|_| {
                let idx: u8 = rng.r#gen::<u8>() % 36;
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'a' + idx - 10) as char
                }
            })
            .collect();
        let key_id = format!("ed25519:{id_chars}");

        Self {
            key_id,
            signing_key,
        }
    }

    /// Reconstruct a keypair from raw private key bytes and a stored key ID.
    ///
    /// Use this on server restart to reload the keypair from the database.
    /// The `private_key` must be the exact 32 bytes returned by
    /// [`private_key_bytes()`](Self::private_key_bytes) when the key was
    /// originally generated. The `key_id` must match the original
    /// (e.g., `ed25519:a1b2c3d4`).
    pub fn from_bytes(key_id: String, private_key: &[u8; 32]) -> Self {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(private_key);
        Self {
            key_id,
            signing_key,
        }
    }

    /// The key ID, e.g. `ed25519:abc12345`.
    ///
    /// This is included in event signatures and federation request headers so
    /// that the verifier knows which public key to use for verification.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The public key as unpadded base64.
    ///
    /// This is the format expected by the `/_matrix/key/v2/server` endpoint,
    /// which remote servers query to obtain this server's public key for
    /// signature verification.
    pub fn public_key_base64(&self) -> String {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
        engine.encode(self.signing_key.verifying_key().as_bytes())
    }

    /// The raw 32-byte private key, for persisting to the database.
    ///
    /// Store these bytes securely. They can be passed back to
    /// [`from_bytes()`](Self::from_bytes) on the next startup to reconstruct
    /// the keypair.
    pub fn private_key_bytes(&self) -> &[u8; 32] {
        self.signing_key.as_bytes()
    }

    /// The raw 32-byte public key.
    ///
    /// Used when you need to pass the public key to [`verify_signature()`] for
    /// local verification, or when constructing key query responses.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Sign arbitrary data and return the Ed25519 signature as unpadded base64.
    ///
    /// In practice, `data` is the canonical JSON bytes of an event (for event
    /// signing) or the canonical JSON of an HTTP request object (for federation
    /// request signing). The returned base64 string is placed into the
    /// `signatures` field of the event or the `Authorization` header.
    pub fn sign(&self, data: &[u8]) -> String {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
        let signature = self.signing_key.sign(data);
        engine.encode(signature.to_bytes())
    }
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyPair")
            .field("key_id", &self.key_id)
            .field("public_key", &self.public_key_base64())
            .finish()
    }
}

/// Verify an Ed25519 signature from a remote server.
///
/// Use this when checking signatures on events received over federation or
/// when verifying federation request `Authorization` headers. You need the
/// remote server's public key bytes (obtained from their
/// `/_matrix/key/v2/server` endpoint), the original message bytes that were
/// signed (canonical JSON), and the base64-encoded signature string.
///
/// Returns `true` if the signature is valid, `false` if it is invalid or if
/// any of the inputs are malformed (bad base64, wrong-length key, etc.).
pub fn verify_signature(public_key_bytes: &[u8; 32], message: &[u8], signature_b64: &str) -> bool {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD_NO_PAD;

    let sig_bytes = match engine.decode(signature_b64) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let sig_array: [u8; 64] = match sig_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return false,
    };

    let verifying_key = match ed25519_dalek::VerifyingKey::from_bytes(public_key_bytes) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let signature = ed25519_dalek::Signature::from_bytes(&sig_array);
    verifying_key.verify(message, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_sign_verify() {
        let kp = KeyPair::generate();
        assert!(kp.key_id().starts_with("ed25519:"));

        let message = b"test message";
        let sig = kp.sign(message);

        assert!(verify_signature(&kp.public_key_bytes(), message, &sig));
    }

    #[test]
    fn test_from_bytes_roundtrip() {
        let kp = KeyPair::generate();
        let restored = KeyPair::from_bytes(kp.key_id().to_string(), kp.private_key_bytes());

        assert_eq!(kp.public_key_base64(), restored.public_key_base64());

        let message = b"roundtrip test";
        let sig = kp.sign(message);
        assert!(verify_signature(
            &restored.public_key_bytes(),
            message,
            &sig
        ));
    }

    #[test]
    fn test_wrong_key_fails_verify() {
        let kp1 = KeyPair::generate();
        let kp2 = KeyPair::generate();

        let sig = kp1.sign(b"hello");
        assert!(!verify_signature(&kp2.public_key_bytes(), b"hello", &sig));
    }

    #[test]
    fn test_wrong_message_fails_verify() {
        let kp = KeyPair::generate();
        let sig = kp.sign(b"hello");
        assert!(!verify_signature(&kp.public_key_bytes(), b"world", &sig));
    }
}
