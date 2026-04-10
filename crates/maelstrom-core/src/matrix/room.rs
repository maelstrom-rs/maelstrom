//! Room-level types that define the rules and structure of Matrix rooms.
//!
//! A Matrix room is a shared state machine.  Its behavior is determined by **state events**
//! (membership, join rules, power levels, history visibility, etc.) that are replicated
//! across all participating homeservers.  This module provides the Rust types for the
//! most important room-level concepts:
//!
//! * [`Membership`] -- the state machine governing each user's relationship to the room.
//! * [`JoinRule`] -- who is allowed to join.
//! * [`HistoryVisibility`] -- who can see past events.
//! * [`PowerLevelContent`] -- the permission system (who can do what).
//! * [`RoomVersion`] -- versioned room format and feature flags.
//! * [`event_type`] -- string constants for all well-known Matrix event types.
//!
//! These types are intentionally simple enums and structs with `parse`/`as_str` round-trip
//! methods, making them easy to serialize to/from the JSON wire format.

use std::fmt;

// ── Membership ──────────────────────────────────────────────────────────

/// A user's membership state in a room -- the core of Matrix's access control model.
///
/// Membership forms a state machine with the following transitions:
///
/// ```text
///                  ┌──────────┐
///        invite    │          │  join (if public/restricted)
///   ┌────────────► │  Invite  ├────────┐
///   │              │          │        │
///   │              └────┬─────┘        ▼
///   │                   │          ┌────────┐
///   │          reject   │          │        │
///   │          (leave)  │          │  Join  │◄──── knock (accepted)
///   │                   │          │        │
/// ┌─┴───┐               ▼          └───┬────┘
/// │     │◄─────── Leave ◄──────────────┘ leave / kick
/// │Leave│
/// │     │──────────────────────────────► Ban
/// └─────┘         ban                    │
///    ▲                                   │
///    └───────────── unban ───────────────┘
/// ```
///
/// * **Join** -- the user is a full participant and can send/receive events.
/// * **Invite** -- the user has been invited but has not yet accepted.
/// * **Leave** -- the user is not in the room (either never joined, left voluntarily,
///   was kicked, or rejected an invite).  This is the default for users with no
///   membership event.
/// * **Ban** -- the user is excluded and cannot rejoin until unbanned.
/// * **Knock** -- the user has requested to join (room versions 7+).  The room's
///   members can then accept (invite) or reject (leave) the knock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Membership {
    /// The user is a full member of the room.
    Join,
    /// The user has been invited but has not yet joined.
    Invite,
    /// The user is not in the room (default state, or left/kicked/rejected).
    Leave,
    /// The user is banned and cannot rejoin until unbanned.
    Ban,
    /// The user has requested to join (pending approval from room members).
    Knock,
}

impl Membership {
    /// Return the wire-format string (`"join"`, `"invite"`, `"leave"`, `"ban"`, `"knock"`).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Join => "join",
            Self::Invite => "invite",
            Self::Leave => "leave",
            Self::Ban => "ban",
            Self::Knock => "knock",
        }
    }

    /// Parse a wire-format string into a `Membership`.  Returns `None` for unrecognized values.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "join" => Some(Self::Join),
            "invite" => Some(Self::Invite),
            "leave" => Some(Self::Leave),
            "ban" => Some(Self::Ban),
            "knock" => Some(Self::Knock),
            _ => None,
        }
    }
}

impl fmt::Display for Membership {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Event type constants ────────────────────────────────────────────────

/// Well-known Matrix event type strings.
///
/// These are `&str` constants rather than an enum because the Matrix spec allows
/// arbitrary custom event types (e.g., `com.example.foo`), and an enum cannot represent
/// an open set.  Handler code compares `pdu.event_type` against these constants.
///
/// The constants are grouped by category:
///
/// ## State events (`state_key` is present)
/// These define the persistent configuration of a room.  Only the most recent event
/// for each `(type, state_key)` pair is "current state".
///
/// ## Timeline / message events (`state_key` is absent)
/// These represent user actions (messages, reactions, redactions, calls) and appear
/// in the room timeline.
///
/// ## Ephemeral events
/// Typing indicators, read receipts, and presence are transient and not stored in the
/// room DAG.
///
/// ## Unstable / MSC events
/// Prefixed with `org.matrix.msc*`, these are experimental features not yet in the
/// stable spec.
pub mod event_type {
    // ── State events ────────────────────────────────────────────────────
    /// The first event in every room; records room version and creator.
    pub const CREATE: &str = "m.room.create";
    /// Tracks a user's membership (join, invite, leave, ban, knock).
    pub const MEMBER: &str = "m.room.member";
    /// Defines permission levels for users and actions.
    pub const POWER_LEVELS: &str = "m.room.power_levels";
    /// Controls who can join the room (public, invite, knock, restricted).
    pub const JOIN_RULES: &str = "m.room.join_rules";
    /// Controls who can see room history.
    pub const HISTORY_VISIBILITY: &str = "m.room.history_visibility";
    /// Human-readable room name.
    pub const NAME: &str = "m.room.name";
    /// Room description / topic.
    pub const TOPIC: &str = "m.room.topic";
    /// Room avatar image (`mxc://` URI).
    pub const AVATAR: &str = "m.room.avatar";
    /// Primary and alternative room aliases (`#name:server`).
    pub const CANONICAL_ALIAS: &str = "m.room.canonical_alias";
    /// Whether guests can join the room.
    pub const GUEST_ACCESS: &str = "m.room.guest_access";
    /// Marks a room as upgraded; points to the replacement room.
    pub const TOMBSTONE: &str = "m.room.tombstone";
    /// Server-level access control list (block/allow servers from participating).
    pub const SERVER_ACL: &str = "m.room.server_acl";
    /// Enables end-to-end encryption (irreversible).
    pub const ENCRYPTION: &str = "m.room.encryption";
    /// List of event IDs pinned to the top of the room.
    pub const PINNED_EVENTS: &str = "m.room.pinned_events";
    /// Invite issued via a third-party identifier (email, phone).
    pub const THIRD_PARTY_INVITE: &str = "m.room.third_party_invite";
    /// Declares a room as a child of a space.
    pub const SPACE_CHILD: &str = "m.space.child";
    /// Declares a room as having a parent space.
    pub const SPACE_PARENT: &str = "m.space.parent";

    // ── Timeline / message events ───────────────────────────────────────
    /// User-visible message (text, image, file, etc.).
    pub const MESSAGE: &str = "m.room.message";
    /// End-to-end encrypted event (wraps another event).
    pub const ENCRYPTED: &str = "m.room.encrypted";
    /// Requests content erasure of another event.
    pub const REDACTION: &str = "m.room.redaction";
    /// Sticker message (image with no text body).
    pub const STICKER: &str = "m.sticker";
    /// Emoji reaction to another event.
    pub const REACTION: &str = "m.reaction";
    /// VoIP call invitation.
    pub const CALL_INVITE: &str = "m.call.invite";

    // ── Ephemeral events (not stored in the room DAG) ───────────────────
    /// Typing indicator (which users are currently typing).
    pub const TYPING: &str = "m.typing";
    /// Read receipt (which events a user has read).
    pub const RECEIPT: &str = "m.receipt";
    /// User online/offline/unavailable status.
    pub const PRESENCE: &str = "m.presence";
    /// Per-room read marker (the "fully read" position).
    pub const FULLY_READ: &str = "m.fully_read";

    // ── Unstable / MSC events ───────────────────────────────────────────
    /// MSC3381: Poll start event.
    pub const POLL_START: &str = "org.matrix.msc3381.poll.start";
    /// MSC3381: Poll response (vote).
    pub const POLL_RESPONSE: &str = "org.matrix.msc3381.poll.response";
    /// MSC3381: Poll end / close.
    pub const POLL_END: &str = "org.matrix.msc3381.poll.end";
}

// ── JoinRule ────────────────────────────────────────────────────────────

/// Controls who is allowed to join a room.
///
/// Set via the `m.room.join_rules` state event.  The join rule determines the room's
/// openness and directly affects the membership state machine transitions.
///
/// * **Public** -- anyone can join without an invite.  Used for open community rooms.
/// * **Invite** -- users must be explicitly invited by a room member with sufficient
///   power level.  The default for private conversations.
/// * **Knock** -- users can request to join ("knock"), and a room member can then
///   accept or reject.  Requires room version 7+.
/// * **Restricted** -- users can join without an invite *if* they are a member of one
///   of the rooms listed in the `allow` conditions (typically a parent space).
///   Requires room version 8+.
/// * **KnockRestricted** -- combines knock and restricted: users can either knock or
///   join via an allow condition.  Requires room version 8+.
/// * **Private** -- nobody can join (effectively a sealed room).  Rarely used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRule {
    /// Anyone can join without an invite.
    Public,
    /// Requires an explicit invite from a room member.
    Invite,
    /// Users can request to join; members approve or reject.
    Knock,
    /// Users can join if they satisfy an allow condition (e.g., membership in a space).
    Restricted,
    /// Combines knock + restricted: knock or satisfy an allow condition.
    KnockRestricted,
    /// Nobody can join.
    Private,
}

impl JoinRule {
    /// Return the wire-format string for this join rule.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Invite => "invite",
            Self::Knock => "knock",
            Self::Restricted => "restricted",
            Self::KnockRestricted => "knock_restricted",
            Self::Private => "private",
        }
    }

    /// Parse a wire-format string.  Returns `None` for unrecognized values.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Self::Public),
            "invite" => Some(Self::Invite),
            "knock" => Some(Self::Knock),
            "restricted" => Some(Self::Restricted),
            "knock_restricted" => Some(Self::KnockRestricted),
            "private" => Some(Self::Private),
            _ => None,
        }
    }

    /// Returns `true` if this join rule requires an explicit invite for a user to join.
    /// This is the case for `Invite`, `Knock`, and `Private` rules.  `Public` and
    /// `Restricted` allow joining without an invite (though restricted rooms require
    /// an allow-condition match).
    pub const fn requires_invite(&self) -> bool {
        matches!(self, Self::Invite | Self::Knock | Self::Private)
    }
}

impl fmt::Display for JoinRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── HistoryVisibility ───────────────────────────────────────────────────

/// Controls who can see room history (past events).
///
/// Set via the `m.room.history_visibility` state event.  This determines whether a user
/// can see events from **before** they joined (or were invited to) the room:
///
/// * **WorldReadable** -- anyone can read the room history, even without joining.
///   Used for fully public rooms (e.g., announcement channels).
/// * **Shared** -- members can see all history from the point the room was created,
///   *including* events from before they joined.  This is the **default** and the most
///   common setting.
/// * **Invited** -- members can see history from the point they were *invited*, but not
///   before.  Events before the invite are hidden.
/// * **Joined** -- members can only see events from the point they actually *joined*.
///   The most restrictive setting for members.
///
/// The [`visible_to_departed`](HistoryVisibility::visible_to_departed) method returns
/// `true` for `WorldReadable` and `Shared` -- the two settings where a user who has
/// *left* the room can still see past events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryVisibility {
    /// Anyone can read the room, even non-members.
    WorldReadable,
    /// Members see history from the point of their invite.
    Invited,
    /// Members see history only from the point they joined.
    Joined,
    /// Members see all history (default).
    #[default]
    Shared,
}

impl HistoryVisibility {
    /// Return the wire-format string for this visibility level.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WorldReadable => "world_readable",
            Self::Invited => "invited",
            Self::Joined => "joined",
            Self::Shared => "shared",
        }
    }

    /// Parse a wire-format string.  Returns `None` for unrecognized values.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "world_readable" => Some(Self::WorldReadable),
            "invited" => Some(Self::Invited),
            "joined" => Some(Self::Joined),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }

    /// Returns `true` if users who have *left* the room can still see past events.
    ///
    /// This is the case for `WorldReadable` (anyone can see) and `Shared` (all members,
    /// including former members, see full history).  For `Invited` and `Joined`, once
    /// the user leaves they lose access to the events.
    pub const fn visible_to_departed(&self) -> bool {
        matches!(self, Self::WorldReadable | Self::Shared)
    }
}

impl fmt::Display for HistoryVisibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── PowerLevelContent ───────────────────────────────────────────────────

/// Parsed representation of `m.room.power_levels` content, with methods for
/// authorization checks.
///
/// **Power levels** are Matrix's permission system.  Every user in a room has an integer
/// power level (default 0), and every action requires a minimum power level.  This struct
/// parses the raw JSON into typed fields and provides convenience methods like
/// [`can_send`](PowerLevelContent::can_send), [`can_ban`](PowerLevelContent::can_ban), etc.
///
/// # Key concepts
///
/// * **`users`** -- explicit power levels for specific users (e.g., `{"@admin:x": 100}`).
/// * **`users_default`** -- power level for users not listed in `users` (default: 0).
/// * **`events`** -- required power level to send specific event types.
/// * **`events_default`** -- required PL to send message events not listed in `events` (default: 0).
/// * **`state_default`** -- required PL to send state events not listed in `events` (default: 50).
/// * **`ban`/`kick`/`invite`/`redact`** -- required PL for these moderation actions.
///
/// Authorization check: `user_level(sender) >= event_level(event_type, is_state)`.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerLevelContent {
    /// Explicit per-user power levels.
    users: std::collections::HashMap<String, i64>,
    /// Default power level for users not in `users`.
    users_default: i64,
    /// Required power levels for specific event types.
    events: std::collections::HashMap<String, i64>,
    /// Required PL to send message events not in `events`.
    events_default: i64,
    /// Required PL to send state events not in `events`.
    state_default: i64,
    /// Required PL to ban a user.
    ban: i64,
    /// Required PL to kick a user.
    kick: i64,
    /// Required PL to invite a user.
    invite: i64,
    /// Required PL to redact another user's events.
    redact: i64,
}

impl PowerLevelContent {
    /// Parse a `PowerLevelContent` from raw `m.room.power_levels` event content JSON.
    ///
    /// Missing fields fall back to spec defaults: `users_default=0`, `events_default=0`,
    /// `state_default=50`, `ban=50`, `kick=50`, `invite=0`, `redact=50`.
    pub fn from_content(content: &serde_json::Value) -> Self {
        let parse_map = |key: &str| -> std::collections::HashMap<String, i64> {
            content
                .get(key)
                .and_then(|u| u.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_i64().map(|pl| (k.clone(), pl)))
                        .collect()
                })
                .unwrap_or_default()
        };
        let int = |key: &str, default: i64| -> i64 {
            content.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
        };
        Self {
            users: parse_map("users"),
            users_default: int("users_default", 0),
            events: parse_map("events"),
            events_default: int("events_default", 0),
            state_default: int("state_default", 50),
            ban: int("ban", 50),
            kick: int("kick", 50),
            invite: int("invite", 0),
            redact: int("redact", 50),
        }
    }

    /// Return the power level for `user_id`.  Falls back to `users_default` (typically 0)
    /// if the user has no explicit entry in the `users` map.
    pub fn user_level(&self, user_id: &str) -> i64 {
        self.users
            .get(user_id)
            .copied()
            .unwrap_or(self.users_default)
    }

    /// Return the required power level to send an event of the given type.
    ///
    /// If the event type has an explicit entry in the `events` map, that value is used.
    /// Otherwise, falls back to `state_default` (50) for state events or `events_default`
    /// (0) for message/timeline events.
    pub fn event_level(&self, event_type: &str, is_state: bool) -> i64 {
        self.events.get(event_type).copied().unwrap_or(if is_state {
            self.state_default
        } else {
            self.events_default
        })
    }

    /// Check whether `user_id` has sufficient power level to send an event of the
    /// given type.  This is the primary authorization check for event creation.
    pub fn can_send(&self, user_id: &str, event_type: &str, is_state: bool) -> bool {
        self.user_level(user_id) >= self.event_level(event_type, is_state)
    }

    /// Check whether `user_id` can ban other users (PL >= `ban`, default 50).
    pub fn can_ban(&self, user_id: &str) -> bool {
        self.user_level(user_id) >= self.ban
    }

    /// Check whether `user_id` can kick other users (PL >= `kick`, default 50).
    pub fn can_kick(&self, user_id: &str) -> bool {
        self.user_level(user_id) >= self.kick
    }

    /// Check whether `user_id` can invite other users (PL >= `invite`, default 0).
    pub fn can_invite(&self, user_id: &str) -> bool {
        self.user_level(user_id) >= self.invite
    }

    /// Check whether `user_id` can redact other users' events (PL >= `redact`, default 50).
    /// Note: users can always redact their *own* events regardless of power level.
    pub fn can_redact(&self, user_id: &str) -> bool {
        self.user_level(user_id) >= self.redact
    }
}

// ── RoomVersion ─────────────────────────────────────────────────────────

/// Matrix room version, which determines the event format, auth rules, state resolution
/// algorithm, and available features for the lifetime of a room.
///
/// A room's version is set once at creation (in the `m.room.create` event) and cannot be
/// changed.  To adopt a new version, the room must be **upgraded** via `m.room.tombstone`,
/// which creates a new room with the desired version and links back to the old one.
///
/// This enum provides **feature introspection methods** that let authorization and
/// federation code branch on version capabilities without hard-coding version numbers:
///
/// * [`event_id_format`](RoomVersion::event_id_format) -- server-generated (v1-v3) vs reference-hash (v4+).
/// * [`state_resolution`](RoomVersion::state_resolution) -- v1 algorithm vs v2 (all versions >= 2).
/// * [`redaction_algorithm`](RoomVersion::redaction_algorithm) -- which fields survive redaction.
/// * [`strict_power_levels`](RoomVersion::strict_power_levels) -- integer-only PLs (v6+).
/// * [`enforce_canonical_json`](RoomVersion::enforce_canonical_json) -- strict JSON (v6+).
/// * [`supports_knock`](RoomVersion::supports_knock) -- knock join rule (v7+).
/// * [`supports_restricted_join`](RoomVersion::supports_restricted_join) -- restricted joins (v8+).
/// * [`has_creator_field`](RoomVersion::has_creator_field) -- `creator` in create content (v1-v10; removed in v11).
///
/// The server currently recognizes versions 1 through 11.  The default for new rooms is
/// [`V10`](RoomVersion::V10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoomVersion {
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    V9,
    V10,
    V11,
}

/// How event IDs are generated for a given room version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventIdFormat {
    /// Versions 1-3: event IDs are assigned by the creating server (e.g., `$abc123:example.com`).
    ServerGenerated,
    /// Versions 4+: event IDs are derived from the reference hash (`$<base64(sha256)>`).
    ReferenceHash,
}

/// Which state resolution algorithm a room version uses.
///
/// State resolution determines how conflicting state is merged when the room DAG forks
/// (i.e., two servers create events concurrently with different prev_events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateResolutionVersion {
    /// Original algorithm (room version 1 only).  Known to have edge cases around
    /// power level changes.
    V1,
    /// Improved algorithm (room version 2+).  Uses topological ordering by depth and
    /// auth chain resolution to produce more predictable results.
    V2,
}

/// Which fields survive a redaction (content erasure).
///
/// When an event is redacted, most of its `content` is stripped.  The algorithm version
/// determines exactly which fields are preserved (e.g., `membership` on member events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionAlgorithm {
    /// Versions 1-10: original set of preserved fields.
    V1,
    /// Version 11+: expanded set of preserved fields (e.g., keeps `join_authorised_via_users_server`).
    V2,
}

impl RoomVersion {
    /// Parse a room version string (e.g., `"10"`) into a `RoomVersion`.
    /// Returns `None` for unrecognized versions.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1" => Some(Self::V1),
            "2" => Some(Self::V2),
            "3" => Some(Self::V3),
            "4" => Some(Self::V4),
            "5" => Some(Self::V5),
            "6" => Some(Self::V6),
            "7" => Some(Self::V7),
            "8" => Some(Self::V8),
            "9" => Some(Self::V9),
            "10" => Some(Self::V10),
            "11" => Some(Self::V11),
            _ => None,
        }
    }

    /// Return the version as a string (e.g., `"10"`).
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "1",
            Self::V2 => "2",
            Self::V3 => "3",
            Self::V4 => "4",
            Self::V5 => "5",
            Self::V6 => "6",
            Self::V7 => "7",
            Self::V8 => "8",
            Self::V9 => "9",
            Self::V10 => "10",
            Self::V11 => "11",
        }
    }

    /// Whether this room version is considered stable by the spec.
    /// All currently defined versions (1-11) are stable.
    pub const fn is_stable(&self) -> bool {
        true
    }

    /// How event IDs are formed.  V1-V3 use server-generated IDs; V4+ use
    /// the reference hash of the event.
    pub const fn event_id_format(&self) -> EventIdFormat {
        match self {
            Self::V1 | Self::V2 | Self::V3 => EventIdFormat::ServerGenerated,
            _ => EventIdFormat::ReferenceHash,
        }
    }

    /// Which state resolution algorithm to use.  Only V1 uses the original
    /// algorithm; all later versions use the improved V2 algorithm.
    pub const fn state_resolution(&self) -> StateResolutionVersion {
        match self {
            Self::V1 => StateResolutionVersion::V1,
            _ => StateResolutionVersion::V2,
        }
    }

    /// Which redaction algorithm to use (determines which content fields survive redaction).
    pub const fn redaction_algorithm(&self) -> RedactionAlgorithm {
        match self {
            Self::V11 => RedactionAlgorithm::V2,
            _ => RedactionAlgorithm::V1,
        }
    }

    /// Whether power level values must be integers (not floats or strings).
    /// V6+ enforce strict integer parsing; earlier versions are lenient.
    pub const fn strict_power_levels(&self) -> bool {
        !matches!(self, Self::V1 | Self::V2 | Self::V3 | Self::V4 | Self::V5)
    }

    /// Whether events must use strict canonical JSON (no duplicate keys, integers
    /// within safe range, etc.).  Enforced in V6+.
    pub const fn enforce_canonical_json(&self) -> bool {
        !matches!(self, Self::V1 | Self::V2 | Self::V3 | Self::V4 | Self::V5)
    }

    /// Whether the `knock` join rule is supported (V7+).
    pub const fn supports_knock(&self) -> bool {
        !matches!(
            self,
            Self::V1 | Self::V2 | Self::V3 | Self::V4 | Self::V5 | Self::V6
        )
    }

    /// Whether `restricted` and `knock_restricted` join rules are supported (V8+).
    pub const fn supports_restricted_join(&self) -> bool {
        !matches!(
            self,
            Self::V1 | Self::V2 | Self::V3 | Self::V4 | Self::V5 | Self::V6 | Self::V7
        )
    }

    /// Whether the `m.room.create` content includes a `creator` field.
    /// V11 removed this field; the event's `sender` is used instead.
    pub const fn has_creator_field(&self) -> bool {
        !matches!(self, Self::V11)
    }

    /// Return a slice of all recognized room versions (V1 through V11).
    pub const fn all() -> &'static [RoomVersion] {
        &[
            Self::V1,
            Self::V2,
            Self::V3,
            Self::V4,
            Self::V5,
            Self::V6,
            Self::V7,
            Self::V8,
            Self::V9,
            Self::V10,
            Self::V11,
        ]
    }

    /// The default room version for newly created rooms (currently V10).
    pub const fn default_version() -> Self {
        Self::V10
    }
}

impl fmt::Display for RoomVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_roundtrip() {
        for m in [
            Membership::Join,
            Membership::Invite,
            Membership::Leave,
            Membership::Ban,
            Membership::Knock,
        ] {
            assert_eq!(Membership::parse(m.as_str()), Some(m));
        }
        assert_eq!(Membership::parse("invalid"), None);
    }

    #[test]
    fn room_version_roundtrip() {
        for v in RoomVersion::all() {
            assert_eq!(RoomVersion::parse(v.as_str()), Some(*v));
        }
        assert_eq!(RoomVersion::parse("99"), None);
    }

    #[test]
    fn version_features() {
        assert!(!RoomVersion::V5.strict_power_levels());
        assert!(RoomVersion::V6.strict_power_levels());
        assert!(!RoomVersion::V6.supports_knock());
        assert!(RoomVersion::V7.supports_knock());
        assert!(RoomVersion::V10.has_creator_field());
        assert!(!RoomVersion::V11.has_creator_field());
    }

    #[test]
    fn power_levels() {
        let pl = PowerLevelContent::from_content(&serde_json::json!({
            "users": {"@admin:x": 100},
            "state_default": 50,
        }));
        assert_eq!(pl.user_level("@admin:x"), 100);
        assert_eq!(pl.user_level("@nobody:x"), 0);
        assert!(pl.can_send("@admin:x", "m.room.power_levels", true));
    }

    #[test]
    fn history_visibility() {
        assert!(HistoryVisibility::Shared.visible_to_departed());
        assert!(!HistoryVisibility::Joined.visible_to_departed());
    }
}
