//! Shared handler utilities -- small functions used across multiple handlers.
//!
//! This module collects helper functions that don't belong to any single spec
//! section but are needed by many handlers: token generation, password hashing,
//! membership checks, and URL encoding.

use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use maelstrom_core::matrix::error::MatrixError;
use maelstrom_storage::traits::StorageError;
use rand::Rng;
use rand::rngs::OsRng;

/// Generate a random access token.
///
/// Produces a 47-character string: the `mat_` prefix followed by 43
/// cryptographically random alphanumeric characters.  The prefix makes
/// it easy to identify leaked tokens in logs or secret scanners.
pub fn generate_access_token() -> String {
    let token: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(43)
        .map(char::from)
        .collect();
    format!("mat_{token}")
}

/// Generate a random session ID for User-Interactive Authentication (UIA) flows.
///
/// UIA is Matrix's multi-step authentication protocol (used during
/// registration, password changes, etc.).  The session ID ties together
/// the steps in a single flow.  24 alphanumeric characters gives plenty
/// of entropy to prevent collisions.
pub fn generate_session_id() -> String {
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

/// Generate a random lowercase localpart for guest or anonymous registrations.
///
/// When a client registers without specifying a `username`, the server
/// auto-assigns one.  This produces a 12-character lowercase alphanumeric
/// string (e.g. `a3bf9xk2m1qw`).
pub fn generate_localpart() -> String {
    let part: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    part.to_lowercase()
}

/// Check that a user has a membership record in a room, returning the
/// membership state string (e.g. `"join"`, `"invite"`, `"leave"`).
///
/// This is the standard access-control gate used before any room operation.
/// Almost every room-scoped handler (send message, get state, read events,
/// set typing, ...) calls this first to verify the user is actually in the
/// room.
///
/// If no membership record exists at all, returns `403 M_FORBIDDEN` with
/// "You are not in this room".  Callers that need a *specific* membership
/// state (e.g. only `join`) should check the returned string themselves:
///
/// ```rust,ignore
/// let membership = require_membership(storage, &user_id, &room_id).await?;
/// if membership != Membership::Join.as_str() {
///     return Err(MatrixError::forbidden("You must be joined to this room"));
/// }
/// ```
pub async fn require_membership(
    storage: &dyn maelstrom_storage::traits::Storage,
    user_id: &str,
    room_id: &str,
) -> Result<String, MatrixError> {
    storage
        .get_membership(user_id, room_id)
        .await
        .map_err(|e| match e {
            StorageError::NotFound => MatrixError::forbidden("You are not in this room"),
            other => crate::extractors::storage_error(other),
        })
}

/// Hash a password using Argon2id with a random salt.
///
/// # Security notes
///
/// - **Argon2id** is a memory-hard KDF, making brute-force attacks expensive
///   even on GPUs.  It's the OWASP-recommended choice for password storage.
/// - Runs on [`tokio::task::spawn_blocking`] because Argon2 is intentionally
///   CPU- and memory-intensive.  Running it on the async executor would stall
///   other tasks on the same Tokio worker thread.
/// - The salt is generated from `OsRng` (OS-level CSPRNG).
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

/// Verify a plaintext password against a stored Argon2id hash.
///
/// Like [`hash_password`], this runs on a blocking thread because Argon2
/// verification is deliberately slow.  Returns `Ok(())` on match or an
/// error string describing the failure.
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

/// Check if a server is allowed by the room's `m.room.server_acl` state event.
///
/// Returns `Ok(())` if the server is allowed (or no ACL exists), or
/// `Err(MatrixError::forbidden)` if the server is denied.
///
/// Rules (per the Matrix spec):
/// 1. If `allow_ip_literals` is false, reject server names that look like IP addresses.
/// 2. Check the `deny` list first -- any match rejects.
/// 3. Check the `allow` list -- the server must match at least one entry.
pub async fn check_server_acl(
    storage: &dyn maelstrom_storage::traits::Storage,
    room_id: &str,
    server_name: &str,
) -> Result<(), MatrixError> {
    use maelstrom_core::matrix::room::event_type as et;

    // Get the m.room.server_acl state event
    let acl_event = match storage.get_state_event(room_id, et::SERVER_ACL, "").await {
        Ok(event) => event,
        Err(_) => return Ok(()), // No ACL = all servers allowed
    };

    let content = &acl_event.content;
    let allow_ip_literals = content
        .get("allow_ip_literals")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let allow = content
        .get("allow")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let deny = content
        .get("deny")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Check IP literal restriction -- strip any port suffix first
    if !allow_ip_literals {
        let host = server_name.split(':').next().unwrap_or(server_name);
        let first_char = host.chars().next().unwrap_or(' ');
        if first_char.is_ascii_digit() || first_char == '[' {
            return Err(MatrixError::forbidden(
                "Server ACL denies IP literal server names",
            ));
        }
    }

    // Check deny list first
    for pattern in &deny {
        if server_acl_glob_match(pattern, server_name) {
            return Err(MatrixError::forbidden("Server is denied by room ACL"));
        }
    }

    // Check allow list
    if allow.is_empty() {
        return Err(MatrixError::forbidden("Server ACL allows no servers"));
    }
    for pattern in &allow {
        if server_acl_glob_match(pattern, server_name) {
            return Ok(());
        }
    }

    Err(MatrixError::forbidden("Server not in room ACL allow list"))
}

/// Simple glob matching for server ACL patterns.
///
/// Supports `*` as a standalone wildcard (matches everything) and `*` as a
/// prefix wildcard (e.g. `*.evil.com` matches `sub.evil.com`).  Literal
/// patterns must match exactly.
fn server_acl_glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    pattern == value
}

/// Find all remote servers that share rooms with a user.
///
/// Iterates over every room the user has joined, collects the server names of
/// all other joined members, and returns the deduplicated set -- excluding the
/// local server. Used to determine which servers need `m.device_list_update` EDUs
/// when a user's device keys change.
pub async fn servers_sharing_rooms(
    storage: &dyn maelstrom_storage::traits::Storage,
    user_id: &str,
    local_server: &str,
) -> Vec<String> {
    let mut servers = std::collections::HashSet::new();
    if let Ok(rooms) = storage.get_joined_rooms(user_id).await {
        for room_id in &rooms {
            if let Ok(members) = storage.get_room_members(room_id, "join").await {
                for member in members {
                    let server = maelstrom_core::matrix::id::server_name_from_sigil_id(&member);
                    if !server.is_empty() && server != local_server {
                        servers.insert(server.to_string());
                    }
                }
            }
        }
    }
    servers.into_iter().collect()
}

/// Percent-encode a string for safe inclusion in URL path segments.
///
/// Used when embedding room IDs, event IDs, or user IDs (which contain
/// `!`, `$`, `@`, `:` characters) into federation request paths.
pub fn percent_encode(input: &str) -> String {
    urlencoding::encode(input).into_owned()
}
