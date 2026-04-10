//! In-memory ephemeral data store — typing indicators and presence.
//!
//! # Why ephemeral?
//!
//! Typing indicators and presence are inherently transient: if a server
//! restarts, it is acceptable (and expected) that all typing indicators clear
//! and presence resets. There is no reason to persist this data to disk or to
//! the database. Keeping it in memory is simpler, faster, and avoids write
//! amplification on hot paths (typing events can fire every few seconds per
//! user per room).
//!
//! # Concurrency model
//!
//! The store uses [`DashMap`] (a concurrent hash map with sharded locks) for
//! both typing and presence. This provides lock-free concurrent reads, which
//! is important because many `/sync` requests read typing/presence state
//! simultaneously, while only occasional writes occur when a user starts
//! typing or changes presence. DashMap avoids the bottleneck of a single
//! `RwLock` contended by hundreds of sync readers.
//!
//! # Single-node vs. cluster mode
//!
//! - **Single-node** ([`EphemeralStore::new`]): writes are purely local. No
//!   gossip channel is created.
//! - **Cluster mode** ([`EphemeralStore::with_gossip`]): returns a store and
//!   an `mpsc::UnboundedReceiver<EphemeralDelta>`. Every local write (typing
//!   change, presence update) emits an [`EphemeralDelta`] on this channel.
//!   A gossip bridge (e.g., chitchat) consumes these deltas and broadcasts
//!   them to other nodes. When a remote delta arrives, the bridge calls
//!   [`merge_typing`](EphemeralStore::merge_typing) or
//!   [`merge_presence`](EphemeralStore::merge_presence), which apply the
//!   state locally WITHOUT emitting a new delta — this prevents infinite
//!   gossip feedback loops.

use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::mpsc;

/// A delta emitted by the local node when ephemeral state changes.
///
/// The gossip bridge consumes these from the channel returned by
/// [`EphemeralStore::with_gossip`] and broadcasts them to peer nodes.
/// Each variant carries enough information for the remote node to
/// reconstruct the state change via the corresponding `merge_*` method.
#[derive(Debug, Clone)]
pub enum EphemeralDelta {
    Typing {
        user_id: String,
        room_id: String,
        typing: bool,
        timeout_ms: u64,
    },
    Presence {
        user_id: String,
        status: String,
        status_msg: Option<String>,
    },
}

/// In-memory ephemeral data store for typing indicators and presence.
///
/// This is the single source of truth for ephemeral state on this node. All
/// `/sync` responses read typing and presence data from here. The store is
/// cheap to create and has zero persistence overhead.
///
/// # Single-node usage
///
/// ```ignore
/// let store = EphemeralStore::new();
/// store.set_typing("@alice:example.com", "!room:example.com", true, 30000);
/// let typers = store.get_typing_users("!room:example.com");
/// ```
///
/// # Cluster usage
///
/// ```ignore
/// let (store, mut rx) = EphemeralStore::with_gossip();
/// // Spawn a task to forward deltas to the gossip layer:
/// tokio::spawn(async move {
///     while let Some(delta) = rx.recv().await {
///         gossip_broadcast(delta).await;
///     }
/// });
/// ```
pub struct EphemeralStore {
    /// Typing state: `room_id -> { user_id -> expires_at }`.
    ///
    /// Typing indicators auto-expire. The inner `Instant` records when the
    /// typing state should be considered stale. Expired entries are pruned
    /// lazily during reads in [`get_typing_users`](Self::get_typing_users).
    typing: DashMap<String, DashMap<String, Instant>>,
    /// Presence state: `user_id -> PresenceRecord`.
    ///
    /// Unlike typing, presence does not expire on a timer — it persists until
    /// explicitly updated (e.g., user goes offline) or the server restarts.
    presence: DashMap<String, PresenceRecord>,
    /// Optional channel for outbound gossip deltas.
    ///
    /// `None` in single-node mode. `Some(tx)` in cluster mode — every local
    /// write sends an [`EphemeralDelta`] through this channel.
    delta_tx: Option<mpsc::UnboundedSender<EphemeralDelta>>,
}

/// Presence state snapshot for a single user.
///
/// Stored in the ephemeral store's presence map and returned to clients
/// via `/sync` responses.
#[derive(Debug, Clone)]
pub struct PresenceRecord {
    pub user_id: String,
    pub status: String,
    pub status_msg: Option<String>,
    pub last_active_ts: u64,
}

impl EphemeralStore {
    /// Create a standalone store for single-node deployments (no gossip).
    ///
    /// All writes are purely local. No delta channel is created.
    pub fn new() -> Self {
        Self {
            typing: DashMap::new(),
            presence: DashMap::new(),
            delta_tx: None,
        }
    }

    /// Create a store wired for cluster gossip.
    ///
    /// Returns the store and an unbounded receiver. The caller should spawn a
    /// task that reads [`EphemeralDelta`] values from the receiver and
    /// broadcasts them to peer nodes via the gossip layer (e.g., chitchat).
    /// The channel is unbounded because typing/presence writes are
    /// low-frequency and must not block the hot path.
    pub fn with_gossip() -> (Self, mpsc::UnboundedReceiver<EphemeralDelta>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let store = Self {
            typing: DashMap::new(),
            presence: DashMap::new(),
            delta_tx: Some(tx),
        };
        (store, rx)
    }

    // ── Typing (local writes — emit delta) ──────────────────────────

    /// Update a user's typing state (local write) and emit a gossip delta.
    ///
    /// Call this when a local user starts or stops typing. The state is
    /// applied immediately to the in-memory store, and if gossip is enabled,
    /// an [`EphemeralDelta::Typing`] is sent to the gossip channel.
    pub fn set_typing(&self, user_id: &str, room_id: &str, typing: bool, timeout_ms: u64) {
        self.apply_typing(user_id, room_id, typing, timeout_ms);

        if let Some(tx) = &self.delta_tx {
            let _ = tx.send(EphemeralDelta::Typing {
                user_id: user_id.to_owned(),
                room_id: room_id.to_owned(),
                typing,
                timeout_ms,
            });
        }
    }

    /// Return the list of currently-typing user IDs in a room.
    ///
    /// Lazily prunes expired entries (those past their timeout) during the
    /// read. This avoids needing a background reaper task. The returned list
    /// is what gets included in the `/sync` response for the room.
    pub fn get_typing_users(&self, room_id: &str) -> Vec<String> {
        let now = Instant::now();

        let Some(room) = self.typing.get(room_id) else {
            return Vec::new();
        };

        // Prune expired entries while collecting active ones.
        let before_count = room.len();
        room.retain(|_, expires_at| *expires_at > now);
        let users: Vec<String> = room.iter().map(|entry| entry.key().clone()).collect();

        if before_count > 0 || !users.is_empty() {
            tracing::debug!(
                room_id = %room_id,
                before = before_count,
                after = users.len(),
                users = ?users,
                "get_typing_users"
            );
        }

        users
    }

    // ── Presence (local writes — emit delta) ────────────────────────

    /// Update a user's presence (local write) and emit a gossip delta.
    ///
    /// Call this when a local user changes their presence status. The state is
    /// applied immediately, and if gossip is enabled, an
    /// [`EphemeralDelta::Presence`] is sent to the gossip channel.
    pub fn set_presence(&self, user_id: &str, status: &str, status_msg: Option<&str>) {
        self.apply_presence(user_id, status, status_msg);

        if let Some(tx) = &self.delta_tx {
            let _ = tx.send(EphemeralDelta::Presence {
                user_id: user_id.to_owned(),
                status: status.to_owned(),
                status_msg: status_msg.map(|s| s.to_owned()),
            });
        }
    }

    /// Look up a user's current presence record, if one exists.
    ///
    /// Returns `None` if the user has never set presence on this node (or
    /// since the last restart). Used by `/sync` to include presence updates.
    pub fn get_presence(&self, user_id: &str) -> Option<PresenceRecord> {
        self.presence.get(user_id).map(|r| r.value().clone())
    }

    // ── Merge (gossip-sourced — no delta emitted) ───────────────────

    /// Merge a remote typing update received from the gossip layer.
    ///
    /// Applies the state change locally but does NOT emit a delta back to the
    /// gossip channel. This prevents feedback loops: node A writes -> gossips
    /// to node B -> node B merges (no gossip back to A).
    pub fn merge_typing(&self, user_id: &str, room_id: &str, typing: bool, timeout_ms: u64) {
        self.apply_typing(user_id, room_id, typing, timeout_ms);
    }

    /// Merge a remote presence update received from the gossip layer.
    ///
    /// Same no-feedback-loop semantics as [`merge_typing`](Self::merge_typing).
    pub fn merge_presence(&self, user_id: &str, status: &str, status_msg: Option<&str>) {
        self.apply_presence(user_id, status, status_msg);
    }

    // ── Internal ────────────────────────────────────────────────────

    fn apply_typing(&self, user_id: &str, room_id: &str, typing: bool, timeout_ms: u64) {
        if typing {
            let expires_at = Instant::now() + std::time::Duration::from_millis(timeout_ms);
            self.typing
                .entry(room_id.to_owned())
                .or_default()
                .insert(user_id.to_owned(), expires_at);
        } else if let Some(room) = self.typing.get(room_id) {
            room.remove(user_id);
        }
    }

    fn apply_presence(&self, user_id: &str, status: &str, status_msg: Option<&str>) {
        let now_ms = super::event::timestamp_ms();

        self.presence.insert(
            user_id.to_owned(),
            PresenceRecord {
                user_id: user_id.to_owned(),
                status: status.to_owned(),
                status_msg: status_msg.map(|s| s.to_owned()),
                last_active_ts: now_ms,
            },
        );
    }
}

impl Default for EphemeralStore {
    fn default() -> Self {
        Self::new()
    }
}
