use serde::{Deserialize, Serialize};
use std::fmt;

/// A Matrix user ID, e.g. `@alice:example.com`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(String);

/// A Matrix room ID, e.g. `!room:example.com`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomId(String);

/// A Matrix event ID, e.g. `$base64hash`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(String);

/// A Matrix device ID, e.g. `ABCDEFG`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

/// A Matrix server name, e.g. `example.com` or `example.com:8448`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerName(String);

/// A Matrix room alias, e.g. `#room:example.com`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RoomAlias(String);

// -- UserId --

impl UserId {
    /// Create a UserId from localpart and server name.
    pub fn new(localpart: &str, server_name: &ServerName) -> Self {
        Self(format!("@{localpart}:{server_name}"))
    }

    /// Parse a UserId from a full string like `@alice:example.com`.
    pub fn parse(s: impl Into<String>) -> Result<Self, IdentifierError> {
        let s = s.into();
        if !s.starts_with('@') || !s.contains(':') {
            return Err(IdentifierError::InvalidFormat {
                kind: "UserId",
                value: s,
            });
        }
        Ok(Self(s))
    }

    /// The localpart (everything between `@` and `:`).
    pub fn localpart(&self) -> &str {
        &self.0[1..self.0.find(':').unwrap_or(self.0.len())]
    }

    /// The server name (everything after the first `:`).
    pub fn server_name(&self) -> &str {
        &self.0[self.0.find(':').unwrap_or(self.0.len()) + 1..]
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -- RoomId --

impl RoomId {
    pub fn parse(s: impl Into<String>) -> Result<Self, IdentifierError> {
        let s = s.into();
        if !s.starts_with('!') || !s.contains(':') {
            return Err(IdentifierError::InvalidFormat {
                kind: "RoomId",
                value: s,
            });
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -- EventId --

impl EventId {
    pub fn parse(s: impl Into<String>) -> Result<Self, IdentifierError> {
        let s = s.into();
        if !s.starts_with('$') {
            return Err(IdentifierError::InvalidFormat {
                kind: "EventId",
                value: s,
            });
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -- DeviceId --

impl DeviceId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string().replace('-', "")[..10].to_uppercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -- ServerName --

impl ServerName {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -- RoomAlias --

impl RoomAlias {
    pub fn parse(s: impl Into<String>) -> Result<Self, IdentifierError> {
        let s = s.into();
        if !s.starts_with('#') || !s.contains(':') {
            return Err(IdentifierError::InvalidFormat {
                kind: "RoomAlias",
                value: s,
            });
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// -- Display impls --

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

/// Errors for identifier parsing.
#[derive(Debug, thiserror::Error)]
pub enum IdentifierError {
    #[error("Invalid {kind} format: {value}")]
    InvalidFormat {
        kind: &'static str,
        value: String,
    },
}
