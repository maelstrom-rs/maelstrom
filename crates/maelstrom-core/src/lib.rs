//! # maelstrom-core
//!
//! The Matrix protocol type library for the Maelstrom homeserver.
//!
//! This crate defines every type, identifier, event structure, error code,
//! signing algorithm, and state-resolution rule that the rest of Maelstrom
//! depends on. It is the single source of truth for "what does the Matrix
//! spec say this thing looks like in Rust?"
//!
//! ## Where everything lives
//!
//! Everything is re-exported through the [`matrix`] module:
//!
//! | Sub-module | What it contains |
//! |------------|-----------------|
//! | [`matrix::id`] | Validated identifier newtypes — `UserId`, `RoomId`, `EventId`, `DeviceId`, `RoomAlias`, `ServerName`. |
//! | [`matrix::event`] | The core `Pdu` (Persistent Data Unit) type — every event that gets stored. |
//! | [`matrix::content`] | Typed event content enums (`RoomCreate`, `RoomMessage`, `RoomMember`, etc.). |
//! | [`matrix::room`] | Room-level enums like `Membership`, `JoinRule`, `RoomVisibility`, `HistoryVisibility`. |
//! | [`matrix::error`] | `MatrixError` and `ErrorCode` — the standard JSON error response from the spec. |
//! | [`matrix::signing`] | Ed25519 signing and verification for events and federation requests. |
//! | [`matrix::state`] | State resolution (the algorithm that decides which events "win" in a room). |
//! | [`matrix::keys`] | Key-related types for device keys, one-time keys, and cross-signing. |
//! | [`matrix::json`] | Canonical JSON helpers used by signing and hashing. |
//! | [`matrix::edu`] | Ephemeral Data Units — typing notifications, read receipts, presence, to-device messages. |
//! | [`matrix::ephemeral`] | Ephemeral event types that ride along in `/sync` but are never persisted. |
//!
//! ## Design philosophy
//!
//! This crate has **zero runtime dependencies**. No HTTP client, no database
//! driver, no async runtime. It is pure types, validation logic, and
//! deterministic algorithms (like canonical JSON and state resolution).
//! That makes it safe to depend on from any layer of the stack — API handlers,
//! federation, storage, tests — without pulling in the world.
//!
//! It replaces [ruma](https://github.com/ruma/ruma) with a simpler,
//! homeserver-optimized set of types. Where ruma tries to cover every
//! possible Matrix client and server scenario with heavy macro usage,
//! maelstrom-core keeps things plain: hand-written structs, derive-based
//! serde, and straightforward validation functions.

pub mod matrix;
