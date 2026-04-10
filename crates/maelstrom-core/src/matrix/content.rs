//! Typed representations of Matrix event `content` payloads.
//!
//! Every Matrix event has an `event_type` string (e.g., `m.room.member`) and a `content`
//! JSON object whose schema is determined by that type.  This module provides Rust structs
//! for each well-known event type so that handler code can work with typed fields instead
//! of raw JSON.
//!
//! # Parse-on-demand pattern
//!
//! Events store their content as `serde_json::Value` (see [`Pdu::content`](super::event::Pdu::content)).
//! Deserialization into a typed struct happens **on demand** when you call
//! [`Pdu::typed_content()`](super::event::Pdu::typed_content) or
//! [`Content::parse()`](Content::parse) directly.  This avoids paying the deserialization
//! cost for events you never inspect, and lets unknown / custom event types
//! (`com.example.custom`) pass through as [`Content::Raw`].
//!
//! # Adding a new event type
//!
//! 1. Define a content struct with `#[derive(Serialize, Deserialize)]`.
//! 2. Add a variant to [`Content`].
//! 3. Add a match arm in [`Content::parse`] mapping the type string to the new variant.

use serde::{Deserialize, Serialize};

use super::room::{HistoryVisibility, JoinRule, Membership, PowerLevelContent};

// ── m.room.create ───────────────────────────────────────────────────────

/// Content of an `m.room.create` event -- the very first event in every room.
///
/// This event is created once when the room is born and is immutable.  It records:
///
/// * **`creator`** -- the user ID that created the room.  Present in room versions 1-10;
///   in v11 the sender field of the event itself serves this purpose and `creator` is
///   removed from the content.
/// * **`room_version`** -- determines the event format, state resolution algorithm, and
///   available features (see [`RoomVersion`](super::room::RoomVersion)).  Defaults to `"1"`.
/// * **`predecessor`** -- if this room was created via a room upgrade (`m.room.tombstone`),
///   this links back to the old room and its tombstone event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContent {
    /// The user who created the room.  `None` in room version 11+ (use the event's
    /// `sender` field instead).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    /// The room version string (e.g., `"10"`).  Determines auth rules, event ID
    /// format, and feature availability for the lifetime of this room.
    #[serde(default = "default_room_version")]
    pub room_version: String,
    /// If this room replaced an older room via upgrade, this references the old room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessor: Option<Predecessor>,
}

fn default_room_version() -> String {
    "1".into()
}

/// A back-reference to the room that was upgraded to create the current room.
///
/// When a room is upgraded (via `m.room.tombstone`), a new room is created with this
/// predecessor link so clients can stitch the two rooms together in their UI and offer
/// to view older history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predecessor {
    /// The room ID of the old (now tombstoned) room.
    pub room_id: String,
    /// The event ID of the `m.room.tombstone` event in the old room.
    pub event_id: String,
}

// ── m.room.member ───────────────────────────────────────────────────────

/// Content of an `m.room.member` event -- the most important state event in Matrix.
///
/// Every user's relationship to a room is tracked by a member event whose `state_key` is
/// the target user ID.  The `membership` field is the core of the Matrix membership state
/// machine (see [`Membership`](super::room::Membership)).
///
/// This event also carries the user's per-room profile (display name and avatar), which
/// lets clients show profile information without a separate profile lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberContent {
    /// The membership state: `"join"`, `"invite"`, `"leave"`, `"ban"`, or `"knock"`.
    /// Drives room access control and determines what the user can see and do.
    pub membership: String,
    /// The user's display name in this room (can differ from their global profile).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub displayname: Option<String>,
    /// The user's avatar as an `mxc://` URI in this room.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// If `true`, the client that created this event considers it a direct message (DM).
    /// Used by clients to separate DMs from group chats in the room list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_direct: Option<bool>,
    /// Human-readable reason for the membership change (e.g., kick/ban reason).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// For restricted room joins (room version 8+): the user ID of a joined member
    /// whose server authorized this join on behalf of the joining user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_authorised_via_users_server: Option<String>,
}

impl MemberContent {
    /// Parse the raw `membership` string into the strongly-typed [`Membership`] enum.
    /// Returns `None` if the string is not a recognized membership value.
    pub fn membership(&self) -> Option<Membership> {
        Membership::parse(&self.membership)
    }
}

// ── m.room.join_rules ───────────────────────────────────────────────────

/// Content of an `m.room.join_rules` event -- controls who is allowed to join the room.
///
/// The `join_rule` field maps to [`JoinRule`](super::room::JoinRule) and determines
/// whether the room is open, invite-only, knock-enabled, or restricted to members of
/// other rooms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRulesContent {
    /// The join rule string: `"public"`, `"invite"`, `"knock"`, `"restricted"`,
    /// `"knock_restricted"`, or `"private"`.
    pub join_rule: String,
    /// For `restricted` or `knock_restricted` rooms: the list of conditions under which
    /// a user may join without an invite (typically membership in another room).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<AllowCondition>>,
}

impl JoinRulesContent {
    /// Parse the raw `join_rule` string into the strongly-typed [`JoinRule`] enum.
    pub fn rule(&self) -> Option<JoinRule> {
        JoinRule::parse(&self.join_rule)
    }
}

/// A single condition in the `allow` list of a restricted/knock_restricted join rule.
///
/// Currently the only defined condition type is `"m.room_membership"`, which grants
/// join access to users who are members of the specified room.  This is how Matrix
/// implements "join this room if you are in the parent space" semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowCondition {
    /// The condition type.  Currently always `"m.room_membership"`.
    #[serde(rename = "type")]
    pub condition_type: String,
    /// The room whose membership satisfies this condition.
    pub room_id: Option<String>,
}

// ── m.room.history_visibility ───────────────────────────────────────────

/// Content of an `m.room.history_visibility` event -- controls whether users can see
/// events from before they joined.
///
/// See [`HistoryVisibility`](super::room::HistoryVisibility) for the meaning of each value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryVisibilityContent {
    /// One of `"world_readable"`, `"shared"`, `"invited"`, or `"joined"`.
    pub history_visibility: String,
}

impl HistoryVisibilityContent {
    /// Parse the raw string into a [`HistoryVisibility`] enum, defaulting to
    /// [`Shared`](HistoryVisibility::Shared) if the value is unrecognized.
    pub fn visibility(&self) -> HistoryVisibility {
        HistoryVisibility::parse(&self.history_visibility).unwrap_or_default()
    }
}

// ── m.room.name / m.room.topic ──────────────────────────────────────────

/// Content of an `m.room.name` event -- sets the human-readable room name.
///
/// The room name is displayed in clients' room lists and headers.  It is a state event
/// with state key `""` (empty string), so there is at most one active name per room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameContent {
    /// The room name.  The spec recommends keeping it under 255 characters.
    #[serde(default)]
    pub name: String,
}

/// Content of an `m.room.topic` event -- sets the room's topic/description.
///
/// Displayed as a subtitle or description in clients.  Like `m.room.name`, this is a
/// state event with state key `""`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicContent {
    /// The room topic text.
    #[serde(default)]
    pub topic: String,
}

// ── m.room.canonical_alias ──────────────────────────────────────────────

/// Content of an `m.room.canonical_alias` event -- maps a human-readable alias to the room.
///
/// Room aliases look like `#general:example.com` and provide a discoverable name for
/// rooms whose IDs (`!random:example.com`) are opaque.  A room can have many aliases but
/// only one canonical alias at a time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalAliasContent {
    /// The primary alias (e.g., `#general:example.com`), or `None` to unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Additional published aliases for this room.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alt_aliases: Vec<String>,
}

// ── m.room.tombstone ────────────────────────────────────────────────────

/// Content of an `m.room.tombstone` event -- signals that this room has been upgraded.
///
/// When a room admin upgrades a room (to adopt a newer room version), the server sends
/// a tombstone event in the old room and creates a new room.  Clients should display
/// the `body` message and redirect users to the `replacement_room`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TombstoneContent {
    /// Human-readable message explaining why the room was upgraded (e.g., "This room
    /// has been upgraded to room version 10").
    pub body: String,
    /// The room ID of the newly created replacement room.
    pub replacement_room: String,
}

// ── m.room.encryption ───────────────────────────────────────────────────

/// Content of an `m.room.encryption` event -- enables end-to-end encryption in the room.
///
/// Once sent, encryption **cannot be disabled** (the spec forbids removing this state
/// event).  All subsequent message events must be encrypted with the specified algorithm.
///
/// The only algorithm currently defined by the spec is `m.megolm.v1.aes-sha2` (Megolm
/// group ratchet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionContent {
    /// The encryption algorithm.  In practice always `"m.megolm.v1.aes-sha2"`.
    pub algorithm: String,
    /// How often to rotate the Megolm session, in milliseconds.
    /// Defaults to one week if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_period_ms: Option<u64>,
    /// How many messages to send before rotating the Megolm session.
    /// Defaults to 100 if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation_period_msgs: Option<u64>,
}

// ── m.room.message ──────────────────────────────────────────────────────

/// Content of an `m.room.message` event -- the workhorse event for user-visible messages.
///
/// Unlike state events, message events are **timeline events** with no `state_key`.  The
/// `msgtype` field determines the sub-type:
///
/// * `m.text` -- plain text message.
/// * `m.image`, `m.video`, `m.audio`, `m.file` -- media attachments.
/// * `m.notice` -- bot/automated messages (clients may style differently).
/// * `m.emote` -- `/me`-style actions.
/// * `m.location` -- geographic location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContent {
    /// The message sub-type (e.g., `m.text`, `m.image`).
    pub msgtype: String,
    /// The plain-text body.  Always present; serves as fallback for clients that
    /// cannot render the formatted version or media.
    #[serde(default)]
    pub body: String,
    /// The format of `formatted_body` (e.g., `"org.matrix.custom.html"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Rich-text body (typically HTML) for clients that support it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted_body: Option<String>,
    /// Relation information for replies, threads, and edits.
    /// Structure varies by relation type; kept as raw JSON for flexibility.
    #[serde(rename = "m.relates_to", skip_serializing_if = "Option::is_none")]
    pub relates_to: Option<serde_json::Value>,
}

// ── m.room.redaction ────────────────────────────────────────────────────

/// Content of an `m.room.redaction` event -- requests erasure of another event's content.
///
/// Redaction does not delete the event from the DAG (that would break federation
/// consistency).  Instead, the target event's `content` is stripped down to only the
/// fields that are structurally necessary (varies by event type), and the redaction
/// event is recorded alongside it.  Clients display redacted events as "[removed]" or
/// similar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionContent {
    /// Optional human-readable reason for the redaction (e.g., "spam").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ── Content enum for exhaustive matching ────────────────────────────────

/// Typed content parsed from a well-known Matrix event type.
///
/// This enum is the result of [`Content::parse`] (or equivalently [`Pdu::typed_content`]).
/// It gives you exhaustive `match` over all event types this server understands, with a
/// [`Raw`](Content::Raw) fallback for everything else.
///
/// # Example
///
/// ```ignore
/// let content = pdu.typed_content();
/// match content {
///     Content::Member(m) => {
///         if m.membership() == Some(Membership::Join) {
///             // handle join
///         }
///     }
///     Content::Name(n) => println!("Room renamed to {}", n.name),
///     Content::Raw(val) => println!("Unknown event content: {val}"),
///     _ => {}
/// }
/// ```
#[derive(Debug, Clone)]
pub enum Content {
    /// `m.room.create` -- room creation event.
    Create(CreateContent),
    /// `m.room.member` -- membership change (join, invite, leave, ban, knock).
    Member(MemberContent),
    /// `m.room.power_levels` -- permission levels for users and actions.
    PowerLevels(PowerLevelContent),
    /// `m.room.join_rules` -- who is allowed to join.
    JoinRules(JoinRulesContent),
    /// `m.room.history_visibility` -- who can see past events.
    HistoryVisibility(HistoryVisibilityContent),
    /// `m.room.name` -- human-readable room name.
    Name(NameContent),
    /// `m.room.topic` -- room description/topic.
    Topic(TopicContent),
    /// `m.room.canonical_alias` -- primary alias (`#name:server`).
    CanonicalAlias(CanonicalAliasContent),
    /// `m.room.tombstone` -- room upgrade marker.
    Tombstone(TombstoneContent),
    /// `m.room.encryption` -- enables E2EE in the room.
    Encryption(EncryptionContent),
    /// `m.room.message` -- user-visible message (text, image, file, etc.).
    Message(MessageContent),
    /// `m.room.redaction` -- content erasure request.
    Redaction(RedactionContent),
    /// Any event type not explicitly handled above.  The raw JSON `content` is preserved
    /// so callers can still inspect it manually.
    Raw(serde_json::Value),
}

impl Content {
    /// Parse raw JSON content into a typed variant based on the event type string.
    ///
    /// If deserialization fails for a known type (e.g., malformed JSON), the content
    /// falls back to [`Content::Raw`] rather than panicking.  This makes the parser
    /// robust against events from buggy or non-conformant homeservers.
    pub fn parse(event_type: &str, raw: &serde_json::Value) -> Self {
        use super::room::event_type as et;
        match event_type {
            et::CREATE => serde_json::from_value(raw.clone())
                .map(Self::Create)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::MEMBER => serde_json::from_value(raw.clone())
                .map(Self::Member)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::POWER_LEVELS => Self::PowerLevels(PowerLevelContent::from_content(raw)),
            et::JOIN_RULES => serde_json::from_value(raw.clone())
                .map(Self::JoinRules)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::HISTORY_VISIBILITY => serde_json::from_value(raw.clone())
                .map(Self::HistoryVisibility)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::NAME => serde_json::from_value(raw.clone())
                .map(Self::Name)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::TOPIC => serde_json::from_value(raw.clone())
                .map(Self::Topic)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::CANONICAL_ALIAS => serde_json::from_value(raw.clone())
                .map(Self::CanonicalAlias)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::TOMBSTONE => serde_json::from_value(raw.clone())
                .map(Self::Tombstone)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::ENCRYPTION => serde_json::from_value(raw.clone())
                .map(Self::Encryption)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::MESSAGE => serde_json::from_value(raw.clone())
                .map(Self::Message)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            et::REDACTION => serde_json::from_value(raw.clone())
                .map(Self::Redaction)
                .unwrap_or_else(|_| Self::Raw(raw.clone())),
            _ => Self::Raw(raw.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_member_content() {
        let raw = serde_json::json!({"membership": "join", "displayname": "Alice"});
        let content = Content::parse("m.room.member", &raw);
        match content {
            Content::Member(m) => {
                assert_eq!(m.membership(), Some(Membership::Join));
                assert_eq!(m.displayname.as_deref(), Some("Alice"));
            }
            _ => panic!("Expected Member"),
        }
    }

    #[test]
    fn parse_create_content() {
        let raw = serde_json::json!({"room_version": "10", "creator": "@alice:example.com"});
        let content = Content::parse("m.room.create", &raw);
        match content {
            Content::Create(c) => {
                assert_eq!(c.room_version, "10");
                assert_eq!(c.creator.as_deref(), Some("@alice:example.com"));
            }
            _ => panic!("Expected Create"),
        }
    }

    #[test]
    fn parse_unknown_falls_to_raw() {
        let raw = serde_json::json!({"custom": true});
        let content = Content::parse("com.example.custom", &raw);
        assert!(matches!(content, Content::Raw(_)));
    }

    #[test]
    fn parse_power_levels() {
        let raw = serde_json::json!({
            "users": {"@admin:x": 100},
            "state_default": 50,
        });
        let content = Content::parse("m.room.power_levels", &raw);
        match content {
            Content::PowerLevels(pl) => {
                assert_eq!(pl.user_level("@admin:x"), 100);
                assert_eq!(pl.user_level("@nobody:x"), 0);
            }
            _ => panic!("Expected PowerLevels"),
        }
    }
}
