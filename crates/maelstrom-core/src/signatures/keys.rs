use ed25519_dalek::{Signer, Verifier};
use rand::Rng;

/// An Ed25519 signing keypair for federation.
#[derive(Clone)]
pub struct KeyPair {
    key_id: String,
    signing_key: ed25519_dalek::SigningKey,
}

impl KeyPair {
    /// Generate a new random Ed25519 keypair with a random key ID.
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

    /// Create a keypair from raw private key bytes and a key ID.
    pub fn from_bytes(key_id: String, private_key: &[u8; 32]) -> Self {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(private_key);
        Self {
            key_id,
            signing_key,
        }
    }

    /// The key ID, e.g. `ed25519:abc12345`.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The public key as unpadded base64.
    pub fn public_key_base64(&self) -> String {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
        engine.encode(self.signing_key.verifying_key().as_bytes())
    }

    /// The raw private key bytes (for storage).
    pub fn private_key_bytes(&self) -> &[u8; 32] {
        self.signing_key.as_bytes()
    }

    /// The raw public key bytes.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Sign data and return the signature as unpadded base64.
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

/// Verify an Ed25519 signature given a public key.
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

        assert!(verify_signature(
            &kp.public_key_bytes(),
            message,
            &sig
        ));
    }

    #[test]
    fn test_from_bytes_roundtrip() {
        let kp = KeyPair::generate();
        let restored = KeyPair::from_bytes(
            kp.key_id().to_string(),
            kp.private_key_bytes(),
        );

        assert_eq!(kp.public_key_base64(), restored.public_key_base64());

        let message = b"roundtrip test";
        let sig = kp.sign(message);
        assert!(verify_signature(&restored.public_key_bytes(), message, &sig));
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
