use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::mpsc;

/// A change event emitted by the local node for gossip propagation.
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
/// Uses `DashMap` for lock-free concurrent reads — many `/sync` readers can
/// query typing/presence without contending with each other or with the
/// occasional writer.
///
/// In cluster mode an optional delta channel forwards local writes to a gossip
/// bridge (chitchat), and `merge_*` methods allow the bridge to inject remote
/// state without re-emitting deltas (preventing feedback loops).
pub struct EphemeralStore {
    /// room_id → { user_id → expires_at }
    typing: DashMap<String, DashMap<String, Instant>>,
    /// user_id → PresenceRecord
    presence: DashMap<String, PresenceRecord>,
    /// Optional channel for outbound gossip deltas.
    delta_tx: Option<mpsc::UnboundedSender<EphemeralDelta>>,
}

/// Presence state for a single user.
#[derive(Debug, Clone)]
pub struct PresenceRecord {
    pub user_id: String,
    pub status: String,
    pub status_msg: Option<String>,
    pub last_active_ts: u64,
}

impl EphemeralStore {
    /// Create a standalone store (single-node, no gossip).
    pub fn new() -> Self {
        Self {
            typing: DashMap::new(),
            presence: DashMap::new(),
            delta_tx: None,
        }
    }

    /// Create a store wired for cluster gossip. Returns the store and a
    /// receiver that the gossip bridge should consume.
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

    pub fn get_typing_users(&self, room_id: &str) -> Vec<String> {
        let now = Instant::now();

        let Some(room) = self.typing.get(room_id) else {
            return Vec::new();
        };

        // Prune expired entries while collecting active ones.
        room.retain(|_, expires_at| *expires_at > now);

        room.iter().map(|entry| entry.key().clone()).collect()
    }

    // ── Presence (local writes — emit delta) ────────────────────────

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

    pub fn get_presence(&self, user_id: &str) -> Option<PresenceRecord> {
        self.presence.get(user_id).map(|r| r.value().clone())
    }

    // ── Merge (gossip-sourced — no delta emitted) ───────────────────

    /// Merge a remote typing update. Does NOT emit a delta.
    pub fn merge_typing(&self, user_id: &str, room_id: &str, typing: bool, timeout_ms: u64) {
        self.apply_typing(user_id, room_id, typing, timeout_ms);
    }

    /// Merge a remote presence update. Does NOT emit a delta.
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
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

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
