use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::Rng;
use rand::rngs::OsRng;

/// Generate a random access token.
pub fn generate_access_token() -> String {
    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(43)
        .map(char::from)
        .collect();
    format!("mat_{token}")
}

/// Generate a random session ID for UIA flows.
pub fn generate_session_id() -> String {
    let id: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();
    id
}

/// Generate a random localpart for users who don't specify a username.
pub fn generate_localpart() -> String {
    let part: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    part.to_lowercase()
}

/// Hash a password using Argon2id.
/// Runs on a blocking thread to avoid stalling the Tokio runtime.
pub async fn hash_password(password: &str) -> Result<String, String> {
    let password = password.to_string();
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| format!("Password hashing failed: {e}"))?;
        Ok(hash.to_string())
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}

/// Verify a password against an Argon2id hash.
/// Runs on a blocking thread to avoid stalling the Tokio runtime.
pub async fn verify_password(password: String, hash: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let parsed_hash =
            PasswordHash::new(&hash).map_err(|e| format!("Invalid password hash: {e}"))?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|e| format!("Password verification failed: {e}"))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))?
}
