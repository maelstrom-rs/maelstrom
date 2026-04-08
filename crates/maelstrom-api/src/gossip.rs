//! Chitchat gossip bridge for cross-node ephemeral state.
//!
//! Typing indicators and presence are published into chitchat's per-node
//! key-value store. Typing keys use `set_with_ttl` so they auto-GC after
//! the cluster's `marked_for_deletion_grace_period`. The value itself
//! carries the epoch-ms expiry so readers can filter accurately even before
//! GC runs.
//!
//! **Key scheme**
//!
//! | Prefix | Key format | Value | Lifecycle |
//! |--------|-----------|-------|-----------|
//! | `t:` | `t:{room_id}:{user_id}` | epoch-ms expiry | `set_with_ttl` / `delete` |
//! | `p:` | `p:{user_id}` | `status\0msg\0ts` | `set` (lives with node) |
//!
//! When a node dies chitchat's failure detector removes it from live nodes,
//! so all its typing and presence keys vanish for free.

use std::sync::Arc;

use chitchat::{ChitchatHandle, ListenerHandle};
use tokio::sync::mpsc;
use tracing::debug;

use maelstrom_core::ephemeral::{EphemeralDelta, EphemeralStore};

use crate::notify::{Notification, Notifier};

const TYPING_PREFIX: &str = "t:";
const PRESENCE_PREFIX: &str = "p:";
/// NUL byte separates fields inside a presence value.
const SEP: char = '\0';

/// Returned handle keeps listener subscriptions alive.  Drop it to stop the
/// gossip bridge.
pub struct GossipBridge {
    _typing_listener: ListenerHandle,
    _presence_listener: ListenerHandle,
}

/// Wire up the gossip bridge.
///
/// - **Outbound**: local `EphemeralDelta`s are published to chitchat.
/// - **Inbound**: `subscribe_event` callbacks merge remote changes into the
///   local `EphemeralStore` and fire the `Notifier` so `/sync` wakes up.
pub async fn start(
    handle: &ChitchatHandle,
    ephemeral: Arc<EphemeralStore>,
    notifier: Arc<dyn Notifier>,
    mut delta_rx: mpsc::UnboundedReceiver<EphemeralDelta>,
) -> GossipBridge {
    let chitchat = handle.chitchat();
    let self_id = handle.chitchat_id().clone();

    // ── Outbound: local deltas → chitchat ───────────────────────────

    let chitchat_out = chitchat.clone();
    tokio::spawn(async move {
        while let Some(delta) = delta_rx.recv().await {
            let mut cc = chitchat_out.lock().await;
            let state = cc.self_node_state();

            match delta {
                EphemeralDelta::Typing {
                    user_id,
                    room_id,
                    typing,
                    timeout_ms,
                } => {
                    let key = format!("{TYPING_PREFIX}{room_id}:{user_id}");
                    if typing {
                        let expires_ms = now_ms() + timeout_ms;
                        state.set_with_ttl(key, expires_ms.to_string());
                    } else {
                        state.delete(&key);
                    }
                }
                EphemeralDelta::Presence {
                    user_id,
                    status,
                    status_msg,
                } => {
                    let key = format!("{PRESENCE_PREFIX}{user_id}");
                    let value = encode_presence(&status, status_msg.as_deref(), now_ms());
                    state.set(key, value);
                }
            }
        }
    });

    // ── Inbound: subscribe to remote key changes ────────────────────

    let cc = chitchat.lock().await;

    // Typing events
    let eph_t = ephemeral.clone();
    let not_t = notifier.clone();
    let self_id_t = self_id.clone();

    let typing_listener = cc.subscribe_event(TYPING_PREFIX, move |evt| {
        if *evt.node == self_id_t {
            return;
        }
        if let Some((room_id, user_id)) = parse_typing_key_suffix(evt.key)
            && let Ok(expires_ms) = evt.value.parse::<u64>()
        {
            let remaining = expires_ms.saturating_sub(now_ms());
            if remaining > 0 {
                eph_t.merge_typing(&user_id, &room_id, true, remaining);
                not_t.notify_sync(Notification::Typing { room_id });
            }
        }
    });

    // Presence events
    let presence_listener = cc.subscribe_event(PRESENCE_PREFIX, move |evt| {
        if *evt.node == self_id {
            return;
        }
        let user_id = evt.key; // key suffix after prefix strip
        if let Some((status, status_msg, _ts)) = decode_presence(evt.value) {
            ephemeral.merge_presence(user_id, &status, status_msg.as_deref());
            notifier.notify_sync(Notification::Presence {
                user_id: user_id.to_string(),
            });
        }
    });

    drop(cc);

    debug!("Gossip bridge started (outbound + inbound event subscriptions)");

    GossipBridge {
        _typing_listener: typing_listener,
        _presence_listener: presence_listener,
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// The key passed to `subscribe_event` callback has the prefix already
/// stripped.  So for typing the suffix is `{room_id}:{user_id}`.
fn parse_typing_key_suffix(suffix: &str) -> Option<(String, String)> {
    let colon = suffix.find(':')?;
    let room_id = &suffix[..colon];
    let user_id = &suffix[colon + 1..];
    if room_id.is_empty() || user_id.is_empty() {
        return None;
    }
    Some((room_id.to_owned(), user_id.to_owned()))
}

fn encode_presence(status: &str, status_msg: Option<&str>, ts: u64) -> String {
    format!("{status}{SEP}{}{SEP}{ts}", status_msg.unwrap_or(""))
}

fn decode_presence(value: &str) -> Option<(String, Option<String>, u64)> {
    let mut parts = value.splitn(3, SEP);
    let status = parts.next()?.to_owned();
    let msg_raw = parts.next()?;
    let ts: u64 = parts.next()?.parse().ok()?;
    let status_msg = if msg_raw.is_empty() {
        None
    } else {
        Some(msg_raw.to_owned())
    };
    Some((status, status_msg, ts))
}
