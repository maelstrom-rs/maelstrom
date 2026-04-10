//! Matrix identifier types — validated newtypes for user IDs, room IDs, etc.
//!
//! Every entity in the Matrix protocol is addressed by a string identifier
//! with a specific format. This module provides a validated newtype for each
//! one, so the rest of the codebase can pass around `UserId` instead of
//! `String` and know at the type level that the value is well-formed.
//!
//! # Sigil identifiers
//!
//! Most Matrix IDs start with a single-character **sigil** that tells you
//! what kind of thing it is:
//!
//! | Sigil | Type | Example |
//! |-------|------|---------|
//! | `@` | User ID | `@alice:example.com` |
//! | `!` | Room ID | `!abc123:example.com` |
//! | `$` | Event ID | `$aGVsbG8...` (base64 hash, v4+ rooms) |
//! | `#` | Room Alias | `#general:example.com` |
//!
//! `ServerName` and `DeviceId` are *not* sigil-prefixed; they have their
//! own simpler formats.
//!
//! # `parse` vs `new` / `generate`
//!
//! Each type offers two ways to construct it:
//!
//! - **`parse(s)`** — validates an existing string (e.g. one received from
//!   a client or federation peer). Returns `Result<Self, IdError>`.
//! - **`new(...)` / `generate(...)`** — builds an ID from known-good parts
//!   (e.g. a localpart you just validated and a server name you own).
//!   Skips validation for speed. Only use these when you control the input.
//!
//! All IDs are capped at 255 bytes per the Matrix spec.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The Matrix spec limits all identifiers to 255 bytes.
const MAX_ID_BYTES: usize = 255;

// ── ServerName ──────────────────────────────────────────────────────────

/// A Matrix server name — the part after the colon in sigil IDs.
///
/// Valid forms (per the spec's "server name" grammar):
/// - **DNS hostname**: `example.com`, `matrix.example.com`
/// - **IPv4 literal**: `1.2.3.4`
/// - **IPv6 literal in brackets**: `[::1]`
/// - Any of the above with an **optional port**: `example.com:8448`, `[::1]:8448`
///
/// # Examples
///
/// ```
/// # use maelstrom_core::matrix::id::ServerName;
/// let name = ServerName::parse("example.com:8448").unwrap();
/// assert_eq!(name.host(), "example.com");
/// assert_eq!(name.port(), Some(8448));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerName(String);

impl ServerName {
    /// Wrap a string as a `ServerName` without validation.
    ///
    /// Use this only when you already know the value is valid (e.g. it
    /// came from your own config or was previously validated). For
    /// untrusted input, use [`parse`](Self::parse).
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Parse and validate a server name string.
    ///
    /// Returns `Err(IdError::Format)` if the string is empty, exceeds
    /// 255 bytes, or doesn't match the server name grammar.
    pub fn parse(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        validate_server_name(&s)?;
        Ok(Self(s))
    }

    /// Borrow the raw string representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Extract just the hostname (stripping the port and IPv6 brackets).
    ///
    /// `"example.com:8448"` -> `"example.com"`, `"[::1]:8448"` -> `"::1"`.
    pub fn host(&self) -> &str {
        host_from_server_name(&self.0)
    }

    /// Extract the port, if one was specified.
    ///
    /// Returns `None` for `"example.com"`, `Some(8448)` for
    /// `"example.com:8448"`.
    pub fn port(&self) -> Option<u16> {
        port_from_server_name(&self.0)
    }
}

// ── UserId ──────────────────────────────────────────────────────────────

/// A Matrix user ID in the format `@localpart:server_name`.
///
/// The **localpart** is the username chosen during registration (e.g.
/// `alice`). The **server_name** is the homeserver that owns the account.
/// Together they form a globally unique address: `@alice:example.com`.
///
/// Localparts are case-sensitive, may contain most printable ASCII, and
/// are at most 255 bytes total (including the `@` and `:server`).
///
/// # Examples
///
/// ```
/// # use maelstrom_core::matrix::id::{UserId, ServerName};
/// // Build from known-good parts (no validation):
/// let server = ServerName::new("example.com");
/// let user = UserId::new("alice", &server);
/// assert_eq!(user.as_str(), "@alice:example.com");
///
/// // Parse from untrusted input (validates):
/// let user = UserId::parse("@bob:matrix.org").unwrap();
/// assert_eq!(user.localpart(), "bob");
/// assert_eq!(user.server_name(), "matrix.org");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(String);

impl UserId {
    /// Build a user ID from a localpart and server name without validation.
    ///
    /// The `@` sigil and `:` separator are added automatically.
    pub fn new(localpart: &str, server_name: &ServerName) -> Self {
        Self(format!("@{localpart}:{server_name}"))
    }

    /// Parse and validate a user ID string.
    ///
    /// Checks: starts with `@`, contains a `:` separator with a non-empty
    /// localpart before it, and total length is at most 255 bytes.
    pub fn parse(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.len() > MAX_ID_BYTES {
            return Err(IdError::TooLong {
                kind: "UserId",
                len: s.len(),
            });
        }
        if !s.starts_with('@') {
            return Err(IdError::Format {
                kind: "UserId",
                value: s,
            });
        }
        let colon = s.find(':').ok_or_else(|| IdError::Format {
            kind: "UserId",
            value: s.clone(),
        })?;
        if colon == 1 {
            return Err(IdError::Format {
                kind: "UserId",
                value: s,
            });
        }
        Ok(Self(s))
    }

    /// The username part without the `@` sigil or `:server_name` suffix.
    ///
    /// `"@alice:example.com"` -> `"alice"`.
    pub fn localpart(&self) -> &str {
        let end = self.0.find(':').unwrap_or(self.0.len());
        &self.0[1..end]
    }

    /// The server name portion of the ID.
    ///
    /// `"@alice:example.com"` -> `"example.com"`.
    pub fn server_name(&self) -> &str {
        server_name_from_sigil_id(&self.0)
    }

    /// Borrow the full ID string including the `@` sigil.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ── RoomId ──────────────────────────────────────────────────────────────

/// A Matrix room ID in the format `!opaque_id:server_name`.
///
/// Room IDs are **not** human-readable. The opaque part is randomly
/// generated by whichever server created the room. Unlike room aliases
/// (`#name:server`), a room ID never changes for the lifetime of a room.
///
/// The server name in a room ID indicates *which server created the room*,
/// not necessarily where the room currently lives (rooms are federated
/// across many servers).
///
/// # Examples
///
/// ```
/// # use maelstrom_core::matrix::id::RoomId;
/// let room = RoomId::parse("!abc123:example.com").unwrap();
/// assert_eq!(room.server_name(), "example.com");
///
/// // Generate a new random room ID for our server:
/// let room = RoomId::generate("myserver.com");
/// assert!(room.as_str().starts_with('!'));
/// assert!(room.as_str().contains(":myserver.com"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomId(String);

impl RoomId {
    /// Generate a fresh random room ID for the given server name.
    ///
    /// Produces `!<random_base64>:<server_name>`. Used when creating a
    /// new room on this homeserver.
    pub fn generate(server_name: &str) -> Self {
        Self(super::event::generate_room_id(server_name))
    }

    /// Parse and validate a room ID string.
    ///
    /// Checks: starts with `!`, contains a `:` separator, total length
    /// is at most 255 bytes.
    pub fn parse(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.len() > MAX_ID_BYTES {
            return Err(IdError::TooLong {
                kind: "RoomId",
                len: s.len(),
            });
        }
        if !s.starts_with('!') || !s.contains(':') {
            return Err(IdError::Format {
                kind: "RoomId",
                value: s,
            });
        }
        Ok(Self(s))
    }

    /// The server name of the homeserver that created this room.
    pub fn server_name(&self) -> &str {
        server_name_from_sigil_id(&self.0)
    }

    /// Borrow the full ID string including the `!` sigil.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ── EventId ─────────────────────────────────────────────────────────────

/// A Matrix event ID in the format `$opaque_id`.
///
/// In room versions 4 and later (which is everything you'll encounter in
/// practice), event IDs are derived from a hash of the event content.
/// The format is `$` followed by URL-safe unpadded base64 of the hash.
///
/// Unlike older room versions, v4+ event IDs have **no server name** — they
/// are purely content-addressed.
///
/// # Examples
///
/// ```
/// # use maelstrom_core::matrix::id::EventId;
/// // Parse an event ID from a client request:
/// let eid = EventId::parse("$aGVsbG8gd29ybGQ").unwrap();
/// assert_eq!(eid.as_str(), "$aGVsbG8gd29ybGQ");
///
/// // Generate a new random event ID:
/// let eid = EventId::generate();
/// assert!(eid.as_str().starts_with('$'));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);

impl EventId {
    /// Generate a random event ID in v4 format (`$` + URL-safe base64).
    ///
    /// Used when this homeserver creates a new event. The actual content
    /// hash is computed separately during signing; this generates the
    /// reference hash placeholder.
    pub fn generate() -> Self {
        Self(super::event::generate_event_id())
    }

    /// Parse and validate an event ID string.
    ///
    /// Checks: starts with `$`, total length is at most 255 bytes.
    pub fn parse(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.len() > MAX_ID_BYTES {
            return Err(IdError::TooLong {
                kind: "EventId",
                len: s.len(),
            });
        }
        if !s.starts_with('$') {
            return Err(IdError::Format {
                kind: "EventId",
                value: s,
            });
        }
        Ok(Self(s))
    }

    /// Borrow the full ID string including the `$` sigil.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ── DeviceId ────────────────────────────────────────────────────────────

/// A Matrix device ID — an opaque identifier for a user's login session.
///
/// Each time a user logs in, they get (or supply) a device ID. It's used
/// for end-to-end encryption key management, to-device messaging, and
/// push notification targeting. Device IDs have **no sigil** and **no
/// server name** — they're just opaque strings scoped to a single user.
///
/// Clients can provide their own device ID at login (to resume a session)
/// or let the server generate one.
///
/// # Examples
///
/// ```
/// # use maelstrom_core::matrix::id::DeviceId;
/// // Server-generated device ID (10-char uppercase hex from a UUID):
/// let device = DeviceId::generate();
/// assert_eq!(device.as_str().len(), 10);
///
/// // Client-supplied device ID:
/// let device = DeviceId::new("MYPHONE_01");
/// assert_eq!(device.as_str(), "MYPHONE_01");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

impl DeviceId {
    /// Wrap an existing device ID string without validation.
    ///
    /// Use this when the client supplied a device ID at login.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Generate a new random device ID.
    ///
    /// Produces a 10-character uppercase hex string derived from a v4 UUID.
    /// This is what's used when a client logs in without specifying a
    /// `device_id`.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string().replace('-', "")[..10].to_uppercase())
    }

    /// Borrow the raw device ID string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ── RoomAlias ───────────────────────────────────────────────────────────

/// A Matrix room alias in the format `#localpart:server_name`.
///
/// Room aliases are the **human-friendly** way to refer to a room. They
/// look like `#general:example.com` and can be created, changed, and
/// deleted without affecting the underlying room ID (`!...`). A single
/// room can have zero, one, or many aliases.
///
/// The server name in the alias indicates which homeserver is
/// *authoritative* for resolving it — that server maintains the mapping
/// from alias to room ID.
///
/// # Examples
///
/// ```
/// # use maelstrom_core::matrix::id::RoomAlias;
/// let alias = RoomAlias::parse("#rust:matrix.org").unwrap();
/// assert_eq!(alias.localpart(), "rust");
/// assert_eq!(alias.server_name(), "matrix.org");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomAlias(String);

impl RoomAlias {
    /// Parse and validate a room alias string.
    ///
    /// Checks: starts with `#`, contains a `:` separator, total length
    /// is at most 255 bytes.
    pub fn parse(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.len() > MAX_ID_BYTES {
            return Err(IdError::TooLong {
                kind: "RoomAlias",
                len: s.len(),
            });
        }
        if !s.starts_with('#') || !s.contains(':') {
            return Err(IdError::Format {
                kind: "RoomAlias",
                value: s,
            });
        }
        Ok(Self(s))
    }

    /// The alias name without the `#` sigil or `:server_name` suffix.
    ///
    /// `"#general:example.com"` -> `"general"`.
    pub fn localpart(&self) -> &str {
        let end = self.0.find(':').unwrap_or(self.0.len());
        &self.0[1..end]
    }

    /// The server name that is authoritative for resolving this alias.
    pub fn server_name(&self) -> &str {
        server_name_from_sigil_id(&self.0)
    }

    /// Borrow the full alias string including the `#` sigil.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ── Display / AsRef ─────────────────────────────────────────────────────

macro_rules! impl_display {
    ($($ty:ty),+) => {
        $(
            impl fmt::Display for $ty {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    write!(f, "{}", self.0)
                }
            }
            impl AsRef<str> for $ty {
                fn as_ref(&self) -> &str {
                    &self.0
                }
            }
        )+
    };
}

impl_display!(UserId, RoomId, EventId, DeviceId, ServerName, RoomAlias);

// ── Server name utilities ───────────────────────────────────────────────

/// Extract the server name from any sigil-prefixed identifier.
///
/// A "sigil identifier" is any Matrix ID that starts with a single
/// character (`@`, `!`, `#`, `$`) followed by a localpart, then `:`,
/// then the server name. This function finds the **first** `:` and
/// returns everything after it.
///
/// This correctly handles server names that themselves contain colons
/// (e.g. `host:8448`), because the localpart never contains a colon:
/// - `@alice:host:8448` -> `"host:8448"`
/// - `!room:example.com` -> `"example.com"`
///
/// Returns an empty string if there is no `:` in the input (malformed ID).
pub fn server_name_from_sigil_id(id: &str) -> &str {
    match id.find(':') {
        Some(pos) if pos + 1 < id.len() => &id[pos + 1..],
        _ => "",
    }
}

/// Validate a string as a Matrix server name.
///
/// The server name grammar (from the spec) allows:
/// - A **DNS hostname** (`example.com`), composed of alphanumeric chars,
///   hyphens, and dots.
/// - An **IPv4 address** (`1.2.3.4`) — validated as a hostname since the
///   character set is the same.
/// - An **IPv6 address in square brackets** (`[::1]`) — must contain at
///   least one `:` inside the brackets.
/// - Any of the above followed by **`:port`** where port is 1-65535.
///
/// The string must be non-empty and at most 255 bytes.
///
/// # When to use this
///
/// You generally don't call this directly — use [`ServerName::parse`]
/// instead, which wraps this and returns a typed `ServerName`. This
/// function exists for cases where you need to validate without
/// constructing the newtype (e.g. validating a server name embedded
/// inside another ID).
pub fn validate_server_name(s: &str) -> Result<(), IdError> {
    if s.is_empty() || s.len() > MAX_ID_BYTES {
        return Err(IdError::Format {
            kind: "ServerName",
            value: s.to_string(),
        });
    }
    if s.starts_with('[') {
        let close = s.find(']').ok_or_else(|| IdError::Format {
            kind: "ServerName",
            value: s.to_string(),
        })?;
        if !s[1..close].contains(':') {
            return Err(IdError::Format {
                kind: "ServerName",
                value: s.to_string(),
            });
        }
        let rest = &s[close + 1..];
        if !rest.is_empty() {
            validate_port_suffix(rest, s)?;
        }
        return Ok(());
    }
    if let Some(last_colon) = s.rfind(':') {
        let maybe_port = &s[last_colon + 1..];
        if !maybe_port.is_empty() && maybe_port.bytes().all(|b| b.is_ascii_digit()) {
            let port: u32 = maybe_port.parse().map_err(|_| IdError::Format {
                kind: "ServerName",
                value: s.to_string(),
            })?;
            if port == 0 || port > 65535 {
                return Err(IdError::Format {
                    kind: "ServerName",
                    value: s.to_string(),
                });
            }
            return validate_host(&s[..last_colon], s);
        }
    }
    validate_host(s, s)
}

/// Validate the `:port` suffix after an IPv6 bracket-close.
fn validate_port_suffix(rest: &str, original: &str) -> Result<(), IdError> {
    if !rest.starts_with(':') {
        return Err(IdError::Format {
            kind: "ServerName",
            value: original.to_string(),
        });
    }
    let port: u32 = rest[1..].parse().map_err(|_| IdError::Format {
        kind: "ServerName",
        value: original.to_string(),
    })?;
    if port == 0 || port > 65535 {
        return Err(IdError::Format {
            kind: "ServerName",
            value: original.to_string(),
        });
    }
    Ok(())
}

/// Validate that a hostname contains only allowed characters
/// (alphanumeric, hyphens, dots) and is non-empty.
fn validate_host(host: &str, original: &str) -> Result<(), IdError> {
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err(IdError::Format {
            kind: "ServerName",
            value: original.to_string(),
        });
    }
    Ok(())
}

/// Extract just the host portion from a server name string, stripping
/// the port (if any) and IPv6 brackets.
///
/// `"example.com:8448"` -> `"example.com"`, `"[::1]:8448"` -> `"::1"`.
pub fn host_from_server_name(s: &str) -> &str {
    if s.starts_with('[') {
        return s.find(']').map(|c| &s[1..c]).unwrap_or(s);
    }
    if let Some(last_colon) = s.rfind(':') {
        let after = &s[last_colon + 1..];
        if !after.is_empty() && after.bytes().all(|b| b.is_ascii_digit()) {
            return &s[..last_colon];
        }
    }
    s
}

/// Extract the port from a server name string, if one is present.
///
/// Returns `None` for `"example.com"`, `Some(8448)` for `"example.com:8448"`.
pub fn port_from_server_name(s: &str) -> Option<u16> {
    if s.starts_with('[') {
        let rest = &s[s.find(']')? + 1..];
        return rest.strip_prefix(':').and_then(|p| p.parse().ok());
    }
    if let Some(last_colon) = s.rfind(':') {
        let after = &s[last_colon + 1..];
        if !after.is_empty() && after.bytes().all(|b| b.is_ascii_digit()) {
            return after.parse().ok();
        }
    }
    None
}

// ── Errors ──────────────────────────────────────────────────────────────

/// Errors returned when parsing a Matrix identifier fails.
///
/// There are exactly two failure modes:
///
/// - **`Format`** — the string doesn't match the expected pattern (wrong
///   sigil, missing colon, empty localpart, invalid server name characters,
///   etc.). The `value` field contains the offending input for diagnostics.
///
/// - **`TooLong`** — the string exceeds the 255-byte limit that the Matrix
///   spec imposes on all identifiers. The `len` field tells you how long
///   the input actually was.
#[derive(Debug, thiserror::Error)]
pub enum IdError {
    /// The identifier's structure is wrong (bad sigil, missing separator, etc.).
    #[error("Invalid {kind} format: {value}")]
    Format { kind: &'static str, value: String },
    /// The identifier exceeds the 255-byte maximum length.
    #[error("{kind} too long ({len} bytes, max {MAX_ID_BYTES})")]
    TooLong { kind: &'static str, len: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_name_variants() {
        assert!(validate_server_name("example.com").is_ok());
        assert!(validate_server_name("example.com:8448").is_ok());
        assert!(validate_server_name("1.2.3.4:8448").is_ok());
        assert!(validate_server_name("[::1]:8448").is_ok());
        assert!(validate_server_name("[::1]").is_ok());
        assert!(validate_server_name("").is_err());
        assert!(validate_server_name(":8448").is_err());
        assert!(validate_server_name("host:0").is_err());
    }

    #[test]
    fn sigil_extraction() {
        assert_eq!(
            server_name_from_sigil_id("@alice:example.com"),
            "example.com"
        );
        assert_eq!(server_name_from_sigil_id("!room:host:8448"), "host:8448");
        assert_eq!(server_name_from_sigil_id("invalid"), "");
    }

    #[test]
    fn user_id_parse() {
        let u = UserId::parse("@alice:example.com").unwrap();
        assert_eq!(u.localpart(), "alice");
        assert_eq!(u.server_name(), "example.com");
        assert!(UserId::parse("alice").is_err());
        assert!(UserId::parse("@:x").is_err());
    }

    #[test]
    fn room_id_server_name() {
        let r = RoomId::parse("!abc:host.docker.internal:53632").unwrap();
        assert_eq!(r.server_name(), "host.docker.internal:53632");
    }

    #[test]
    fn host_port_extraction() {
        assert_eq!(host_from_server_name("example.com:8448"), "example.com");
        assert_eq!(port_from_server_name("example.com:8448"), Some(8448));
        assert_eq!(host_from_server_name("[::1]:8448"), "::1");
        assert_eq!(port_from_server_name("[::1]"), None);
    }
}
