//! Ephemeral Data Units (EDUs) — transient events exchanged between servers.
//!
//! # What are EDUs?
//!
//! While PDUs (Persistent Data Units) are the events that form the room DAG
//! and are stored permanently (messages, state changes, etc.), EDUs are
//! short-lived signals that do NOT persist in the DAG. They carry real-time
//! information like:
//!
//! - **Typing indicators** — "Alice is typing in room X"
//! - **Presence updates** — "Bob is online / idle / offline"
//! - **Read receipts** — "Carol has read up to event Y"
//! - **Device list updates** — "Dave added a new device" (for E2EE key tracking)
//! - **Direct-to-device messages** — encrypted key shares sent to specific devices
//!
//! EDUs are sent between homeservers inside federation transactions (alongside
//! PDUs), but they are fire-and-forget: if a server misses an EDU, it is not
//! retried. This is acceptable because EDUs represent ephemeral state —
//! a missed typing notification simply means the indicator appears slightly
//! late or not at all.
//!
//! # Structure
//!
//! The [`Edu`] struct is a generic container with an `edu_type` string and
//! opaque JSON `content`. Use [`Edu::typed()`] to parse the content into one
//! of the well-known types ([`TypingEdu`], [`PresenceEdu`], etc.). Unknown
//! EDU types are preserved as [`EduContent::Unknown`] so forward-compatible
//! federation transactions do not break on new EDU types.

use serde::{Deserialize, Serialize};

/// An EDU sent over federation in a transaction.
///
/// This is the wire format: an `edu_type` string identifying the kind of EDU
/// (e.g., `"m.typing"`, `"m.presence"`) and opaque JSON `content`. Call
/// [`typed()`](Self::typed) to deserialize the content into a strongly-typed
/// variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edu {
    pub edu_type: String,
    pub content: serde_json::Value,
}

/// Typed EDU content, parsed from the generic [`Edu`] container.
///
/// Each variant corresponds to a well-known EDU type. Unknown types are
/// preserved as [`Unknown`](Self::Unknown) with the raw JSON content, so the
/// server can forward them or log them without failing.
#[derive(Debug, Clone)]
pub enum EduContent {
    Typing(TypingEdu),
    Presence(PresenceEdu),
    Receipt(ReceiptEdu),
    DeviceListUpdate(DeviceListUpdateEdu),
    DirectToDevice(DirectToDeviceEdu),
    Unknown(serde_json::Value),
}

impl Edu {
    /// Parse the opaque JSON content into a strongly-typed EDU variant.
    ///
    /// Matches on `edu_type` and attempts to deserialize `content` into the
    /// corresponding struct. If deserialization fails (e.g., a remote server
    /// sent a malformed EDU), falls back to [`EduContent::Unknown`] rather
    /// than returning an error — EDUs are best-effort and should not break
    /// transaction processing.
    pub fn typed(&self) -> EduContent {
        match self.edu_type.as_str() {
            "m.typing" => serde_json::from_value(self.content.clone())
                .map(EduContent::Typing)
                .unwrap_or(EduContent::Unknown(self.content.clone())),
            "m.presence" => serde_json::from_value(self.content.clone())
                .map(EduContent::Presence)
                .unwrap_or(EduContent::Unknown(self.content.clone())),
            "m.receipt" => serde_json::from_value(self.content.clone())
                .map(EduContent::Receipt)
                .unwrap_or(EduContent::Unknown(self.content.clone())),
            "m.device_list_update" => serde_json::from_value(self.content.clone())
                .map(EduContent::DeviceListUpdate)
                .unwrap_or(EduContent::Unknown(self.content.clone())),
            "m.direct_to_device" => serde_json::from_value(self.content.clone())
                .map(EduContent::DirectToDevice)
                .unwrap_or(EduContent::Unknown(self.content.clone())),
            _ => EduContent::Unknown(self.content.clone()),
        }
    }
}

/// A typing notification EDU (`m.typing`).
///
/// Sent by a remote server when one of its users starts or stops typing in a
/// room. The receiving server updates its ephemeral typing state and includes
/// the change in the next `/sync` response for users in that room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypingEdu {
    /// The room the user is typing in.
    pub room_id: String,
    /// The user who started or stopped typing.
    pub user_id: String,
    /// `true` if the user is currently typing, `false` if they stopped.
    pub typing: bool,
}

/// A presence update EDU (`m.presence`).
///
/// Sent when a remote user's presence changes (online, offline, unavailable).
/// Presence is per-user, not per-room, so this EDU does not have a `room_id`.
/// The receiving server stores the presence state in its ephemeral store and
/// delivers it to local users who share a room with the remote user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceEdu {
    /// The user whose presence changed.
    pub user_id: String,
    /// One of `"online"`, `"offline"`, or `"unavailable"`.
    pub presence: String,
    /// Optional human-readable status message (e.g., "In a meeting").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_msg: Option<String>,
    /// Milliseconds since the user was last active, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_ago: Option<u64>,
    /// Whether the user is currently active on a client right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currently_active: Option<bool>,
}

/// A read receipt EDU (`m.receipt`).
///
/// Sent when a remote user reads events in a room. The `event_ids` field
/// contains the event(s) the user has read up to. Receipt types include
/// `"m.read"` (public read marker) and `"m.read.private"` (private, only
/// visible to the sender). The receiving server persists this receipt and
/// delivers it via `/sync`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptEdu {
    /// The room containing the read events.
    pub room_id: String,
    /// The receipt type, e.g. `"m.read"` or `"m.read.private"`.
    #[serde(rename = "type")]
    pub receipt_type: String,
    /// The user who sent the receipt.
    pub user_id: String,
    /// The event ID(s) the user has read up to.
    pub event_ids: Vec<String>,
}

/// A device list update EDU (`m.device_list_update`).
///
/// Sent when a remote user adds, removes, or modifies a device. This is
/// critical for end-to-end encryption: local users who share encrypted rooms
/// with the remote user need to know about device changes so they can update
/// their Olm/Megolm sessions accordingly. The `stream_id` and `prev_id`
/// fields allow servers to detect gaps and request full device list resync
/// if updates were missed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceListUpdateEdu {
    /// The user whose device list changed.
    pub user_id: String,
    /// The device that was added, updated, or removed.
    pub device_id: String,
    /// Human-readable device name, if set by the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_display_name: Option<String>,
    /// The device's E2EE keys (Curve25519 identity key, Ed25519 signing key, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<serde_json::Value>,
    /// Monotonically increasing stream position for ordering device list updates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<i64>,
    /// The `stream_id` values this update supersedes. Used to detect missed updates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_id: Option<Vec<i64>>,
    /// `true` if the device was removed (deleted).
    #[serde(default)]
    pub deleted: bool,
}

/// A direct-to-device message EDU (`m.direct_to_device`).
///
/// Used to deliver messages directly to specific devices, bypassing the room
/// DAG entirely. The primary use case is E2EE key sharing: when a user joins
/// an encrypted room, existing members send Megolm session keys to the new
/// user's devices via direct-to-device messages. The `messages` field is a
/// nested map of `{ user_id: { device_id: content } }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectToDeviceEdu {
    /// The user who sent the direct-to-device message.
    pub sender: String,
    /// The event type, e.g. `"m.room_key"` for Megolm key shares.
    #[serde(rename = "type")]
    pub event_type: String,
    /// A unique ID for deduplication (the sender generates this).
    pub message_id: String,
    /// Nested map: `{ user_id: { device_id: content } }`. Each device gets
    /// its own content (typically an individually encrypted key payload).
    pub messages: serde_json::Value,
}
