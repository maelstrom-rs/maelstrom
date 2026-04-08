use serde::{Deserialize, Serialize};

/// A stored event — the core unit of data in Matrix.
///
/// Includes both local fields (stream_position) and federation fields
/// (signatures, hashes, auth_events, prev_events, origin, depth).
/// Federation fields are `Option` for backward compatibility with locally-created events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEvent {
    pub event_id: String,
    pub room_id: String,
    pub sender: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_key: Option<String>,
    pub content: serde_json::Value,
    pub origin_server_ts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsigned: Option<serde_json::Value>,
    /// Monotonic position for sync ordering. Not part of the Matrix event format.
    #[serde(skip_serializing)]
    pub stream_position: i64,

    // -- Federation fields (Phase 7) --

    /// Origin server name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Auth event IDs that authorize this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_events: Option<Vec<String>>,
    /// Previous event IDs in the DAG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_events: Option<Vec<String>>,
    /// Depth in the event DAG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
    /// Content hashes, e.g. `{"sha256": "..."}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hashes: Option<serde_json::Value>,
    /// Signatures, e.g. `{"server.name": {"ed25519:key_id": "sig..."}}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signatures: Option<serde_json::Value>,
}

impl StoredEvent {
    /// Convert to the client-facing event format (for sync/messages responses).
    pub fn to_client_event(&self) -> serde_json::Value {
        let mut event = serde_json::json!({
            "event_id": self.event_id,
            "room_id": self.room_id,
            "sender": self.sender,
            "type": self.event_type,
            "content": self.content,
            "origin_server_ts": self.origin_server_ts,
        });

        if let Some(sk) = &self.state_key {
            event["state_key"] = serde_json::Value::String(sk.clone());
        }

        if let Some(unsigned) = &self.unsigned {
            event["unsigned"] = unsigned.clone();
        }

        event
    }

    /// Convert to the federation PDU format (for server-to-server API).
    pub fn to_federation_event(&self) -> serde_json::Value {
        let mut event = self.to_client_event();

        if let Some(origin) = &self.origin {
            event["origin"] = serde_json::Value::String(origin.clone());
        }
        if let Some(auth) = &self.auth_events {
            event["auth_events"] = serde_json::json!(auth);
        }
        if let Some(prev) = &self.prev_events {
            event["prev_events"] = serde_json::json!(prev);
        }
        if let Some(d) = self.depth {
            event["depth"] = serde_json::json!(d);
        }
        if let Some(h) = &self.hashes {
            event["hashes"] = h.clone();
        }
        if let Some(s) = &self.signatures {
            event["signatures"] = s.clone();
        }

        event
    }

    /// Whether this is a state event.
    pub fn is_state(&self) -> bool {
        self.state_key.is_some()
    }
}

/// Generate a random event ID in v4 format: `$` + url-safe base64.
pub fn generate_event_id() -> String {
    use rand::Rng;
    let bytes: [u8; 18] = rand::thread_rng().r#gen();
    let encoded: String = bytes
        .iter()
        .map(|b| {
            let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
            chars[(b % 64) as usize] as char
        })
        .collect();
    format!("${encoded}")
}

/// Generate a random room ID: `!` + random + `:` + server_name.
pub fn generate_room_id(server_name: &str) -> String {
    use rand::Rng;
    let chars: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(18)
        .map(char::from)
        .collect();
    format!("!{chars}:{server_name}")
}

/// Current time in milliseconds since Unix epoch.
pub fn timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Default power levels content for a new room.
pub fn default_power_levels(creator: &str) -> serde_json::Value {
    serde_json::json!({
        "ban": 50,
        "events_default": 0,
        "invite": 0,
        "kick": 50,
        "redact": 50,
        "state_default": 50,
        "users_default": 0,
        "events": {
            "m.room.name": 50,
            "m.room.power_levels": 100,
            "m.room.history_visibility": 100,
            "m.room.canonical_alias": 50,
            "m.room.avatar": 50,
            "m.room.tombstone": 100,
            "m.room.server_acl": 100,
            "m.room.encryption": 100
        },
        "users": {
            creator: 100
        }
    })
}
