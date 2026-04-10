//! State Resolution v2 — resolving conflicting room state during federation.
//!
//! # What is state resolution?
//!
//! Matrix rooms are replicated across multiple homeservers. Because servers can
//! process events concurrently (or while partitioned), it is possible for two
//! servers to have different values for the same piece of room state. For
//! example, server A might think the room name is "General" while server B
//! thinks it is "Announcements" — each set the name while the other was
//! unreachable.
//!
//! State resolution is the deterministic algorithm that takes these conflicting
//! state sets and produces a single, agreed-upon resolved state. Every server
//! that runs the same algorithm on the same inputs will arrive at the same
//! result, which is how Matrix achieves eventual consistency without a central
//! authority.
//!
//! # When does this run?
//!
//! State resolution runs during federation when a server receives events that
//! branch the room DAG — i.e., the incoming event has `prev_events` that
//! reference a different fork than the server's current state. The server
//! collects the state at each fork tip and feeds them into [`resolve_state`].
//!
//! # How the algorithm works (simplified)
//!
//! 1. **Unconflicted state** — for each `(event_type, state_key)` pair, if
//!    every state set agrees on the same event (same `event_id`), that state
//!    is unconflicted and goes straight into the result.
//!
//! 2. **Conflicted events** — entries where the state sets disagree are split
//!    into two groups:
//!    - **Auth-related events** — `m.room.create`, `m.room.power_levels`,
//!      `m.room.join_rules`, `m.room.member`, `m.room.third_party_invite`.
//!      These are resolved first because they determine *who is allowed to do
//!      what* — other state events depend on them.
//!    - **Non-auth events** — everything else (room name, topic, etc.).
//!
//! 3. **Sort order** — conflicted events within each group are sorted by:
//!    - **Power level descending** — events from higher-power users win. An
//!      admin's name change beats a regular user's.
//!    - **Timestamp ascending** — among users with equal power, the earlier
//!      event wins. This biases toward the "first write" in a conflict.
//!    - **Event ID ascending** — lexicographic tiebreaker for determinism
//!      when power and timestamp are identical.
//!
//! 4. **Iterative auth check** — each conflicted event is checked against the
//!    partial resolved state built so far. If the sender has sufficient power
//!    level to send that event type (according to the current resolved power
//!    levels), the event is accepted into the resolved state. Otherwise it is
//!    discarded.
//!
//! See: <https://spec.matrix.org/latest/rooms/v2/>

use std::collections::{HashMap, HashSet};

use super::event::Pdu;

/// A state key tuple: `(event_type, state_key)`.
///
/// Each piece of room state is uniquely identified by this pair. For example,
/// `("m.room.name", "")` is the room name, and `("m.room.member", "@alice:example.com")`
/// is Alice's membership in the room.
type StateKey = (String, String);

/// Resolve conflicting room state using the v2 algorithm.
///
/// # Arguments
///
/// - `state_sets` — one state map per fork of the room DAG. Each map is
///   `(event_type, state_key) -> Pdu`. When a room has two fork tips, you
///   pass two state sets representing the state at each tip.
/// - `_auth_events` — reserved for full mainline ordering (not yet used in
///   this simplified implementation). Pass an empty map.
///
/// # Returns
///
/// A single resolved state map that all servers will agree on, given the same
/// inputs. The algorithm is fully deterministic.
///
/// # Algorithm overview
///
/// 1. Unconflicted state passes through unchanged.
/// 2. Auth-related conflicts are sorted by (power desc, timestamp asc,
///    event_id asc) and applied iteratively with auth checks.
/// 3. Non-auth conflicts are sorted the same way and applied against the
///    state built so far (which now includes resolved auth events).
///
/// See the module docs for a detailed walkthrough.
pub fn resolve_state(
    state_sets: &[HashMap<StateKey, Pdu>],
    _auth_events: &HashMap<String, Pdu>,
) -> HashMap<StateKey, Pdu> {
    if state_sets.is_empty() {
        return HashMap::new();
    }

    if state_sets.len() == 1 {
        return state_sets[0].clone();
    }

    // Step 1: Compute the unconflicted state (events present in all sets with the same event_id).
    let mut unconflicted = HashMap::new();
    let mut conflicted: HashMap<StateKey, Vec<Pdu>> = HashMap::new();

    // Collect all state keys across all sets.
    let mut all_keys: HashSet<StateKey> = HashSet::new();
    for set in state_sets {
        for key in set.keys() {
            all_keys.insert(key.clone());
        }
    }

    for key in &all_keys {
        let mut event_ids: Vec<Option<&str>> = Vec::new();
        let mut events: Vec<&Pdu> = Vec::new();

        for set in state_sets {
            if let Some(event) = set.get(key) {
                event_ids.push(Some(&event.event_id));
                events.push(event);
            } else {
                event_ids.push(None);
            }
        }

        // Check if all sets agree on the same event_id (or are absent).
        let non_none: Vec<&str> = event_ids.iter().filter_map(|e| *e).collect();
        if non_none.is_empty() {
            continue;
        }

        let all_same =
            non_none.windows(2).all(|w| w[0] == w[1]) && non_none.len() == state_sets.len();

        if all_same {
            // Unconflicted — all agree.
            unconflicted.insert(key.clone(), events[0].clone());
        } else {
            // Conflicted — different events for the same state key.
            let unique_events: Vec<Pdu> = events.iter().map(|e| (*e).clone()).collect::<Vec<_>>();
            conflicted.insert(key.clone(), unique_events);
        }
    }

    // Step 2: For the conflicted events, separate into auth-related and non-auth-related.
    let auth_types: HashSet<&str> = [
        "m.room.create",
        "m.room.power_levels",
        "m.room.join_rules",
        "m.room.member",
        "m.room.third_party_invite",
    ]
    .iter()
    .copied()
    .collect();

    let mut conflicted_auth = Vec::new();
    let mut conflicted_other = Vec::new();

    for (key, events) in &conflicted {
        if auth_types.contains(key.0.as_str()) {
            for event in events {
                conflicted_auth.push(event.clone());
            }
        } else {
            for event in events {
                conflicted_other.push(event.clone());
            }
        }
    }

    // Step 3: Sort auth-related conflicted events by (power_level, origin_server_ts, event_id).
    // This is the "reverse topological power ordering" simplified.
    let power_levels = unconflicted
        .get(&("m.room.power_levels".to_string(), String::new()))
        .map(|e| &e.content);

    conflicted_auth.sort_by(|a, b| {
        let pa = get_sender_power_level(&a.sender, power_levels);
        let pb = get_sender_power_level(&b.sender, power_levels);
        pb.cmp(&pa) // Higher power level first
            .then(a.origin_server_ts.cmp(&b.origin_server_ts)) // Earlier timestamp first
            .then(a.event_id.cmp(&b.event_id)) // Lexicographic tiebreaker
    });

    // Step 4: Iteratively apply auth events — for each auth event in order,
    // check if it's allowed by the current partial state, and if so, include it.
    let mut resolved = unconflicted.clone();

    for event in &conflicted_auth {
        let key = (
            event.event_type.clone(),
            event.state_key.clone().unwrap_or_default(),
        );
        if !resolved.contains_key(&key) && is_auth_allowed(event, &resolved) {
            resolved.insert(key, event.clone());
        }
        // If key already set by a higher-priority event, skip.
    }

    // Step 5: Sort non-auth conflicted events and apply the same logic.
    conflicted_other.sort_by(|a, b| {
        let pa = get_sender_power_level(&a.sender, power_levels);
        let pb = get_sender_power_level(&b.sender, power_levels);
        pb.cmp(&pa)
            .then(a.origin_server_ts.cmp(&b.origin_server_ts))
            .then(a.event_id.cmp(&b.event_id))
    });

    for event in &conflicted_other {
        let key = (
            event.event_type.clone(),
            event.state_key.clone().unwrap_or_default(),
        );
        if !resolved.contains_key(&key) {
            // No existing event — accept if auth passes.
            if is_auth_allowed(event, &resolved) {
                resolved.insert(key, event.clone());
            }
        }
        // If key already set by a higher-priority event, skip.
    }

    resolved
}

/// Get the power level of a sender from the `m.room.power_levels` event content.
///
/// Looks up the sender in `content.users.<sender>` first, then falls back to
/// `content.users_default`, then falls back to 0. This matches the Matrix
/// spec's power level resolution rules.
fn get_sender_power_level(sender: &str, power_levels: Option<&serde_json::Value>) -> i64 {
    if let Some(pl) = power_levels {
        if let Some(level) = pl
            .get("users")
            .and_then(|u| u.get(sender))
            .and_then(|l| l.as_i64())
        {
            return level;
        }
        if let Some(default) = pl.get("users_default").and_then(|d| d.as_i64()) {
            return default;
        }
    }
    0
}

/// Simplified auth check: does the sender have sufficient power to send this event?
///
/// Checks the sender's power level against the required level for the event
/// type. For state events, the required level comes from `content.events.<type>`
/// or `content.state_default` (default 50). For non-state events, it comes
/// from `content.events.<type>` or `content.events_default` (default 0).
///
/// If no power levels event exists in the current state yet (early room
/// bootstrap), all events are allowed (required level = 0).
fn is_auth_allowed(event: &Pdu, current_state: &HashMap<StateKey, Pdu>) -> bool {
    let power_levels = current_state
        .get(&("m.room.power_levels".to_string(), String::new()))
        .map(|e| &e.content);

    let sender_level = get_sender_power_level(&event.sender, power_levels);

    // Get the required level for this event type
    let required_level = if event.state_key.is_some() {
        // State events default to state_default (50)
        if let Some(pl) = power_levels {
            if let Some(level) = pl
                .get("events")
                .and_then(|e| e.get(&event.event_type))
                .and_then(|l| l.as_i64())
            {
                level
            } else {
                pl.get("state_default")
                    .and_then(|d| d.as_i64())
                    .unwrap_or(50)
            }
        } else {
            0 // No power levels yet, allow everything
        }
    } else {
        // Non-state events default to events_default (0)
        if let Some(pl) = power_levels {
            pl.get("events_default")
                .and_then(|d| d.as_i64())
                .unwrap_or(0)
        } else {
            0
        }
    };

    sender_level >= required_level
}

#[cfg(test)]
mod tests {
    use super::room::event_type as et;
    use super::*;

    fn make_event(event_id: &str, event_type: &str, state_key: &str, sender: &str, ts: u64) -> Pdu {
        Pdu {
            event_id: event_id.to_string(),
            room_id: "!room:example.com".to_string(),
            sender: sender.to_string(),
            event_type: event_type.to_string(),
            state_key: Some(state_key.to_string()),
            content: serde_json::json!({}),
            origin_server_ts: ts,
            unsigned: None,
            stream_position: 0,
            origin: None,
            auth_events: None,
            prev_events: None,
            depth: None,
            hashes: None,
            signatures: None,
        }
    }

    #[test]
    fn test_resolve_single_set() {
        let mut state = HashMap::new();
        state.insert(
            ("m.room.create".to_string(), String::new()),
            make_event("$create", et::CREATE, "", "@alice:a.com", 1000),
        );

        let resolved = resolve_state(&[state.clone()], &HashMap::new());
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[&("m.room.create".to_string(), String::new())].event_id,
            "$create"
        );
    }

    #[test]
    fn test_resolve_unconflicted() {
        let create = make_event("$create", et::CREATE, "", "@alice:a.com", 1000);
        let name = make_event("$name", et::NAME, "", "@alice:a.com", 2000);

        let mut set1 = HashMap::new();
        set1.insert(("m.room.create".to_string(), String::new()), create.clone());
        set1.insert(("m.room.name".to_string(), String::new()), name.clone());

        let mut set2 = HashMap::new();
        set2.insert(("m.room.create".to_string(), String::new()), create);
        set2.insert(("m.room.name".to_string(), String::new()), name);

        let resolved = resolve_state(&[set1, set2], &HashMap::new());
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn test_resolve_conflicted_picks_earlier_timestamp() {
        let name1 = make_event("$name1", et::NAME, "", "@alice:a.com", 1000);
        let name2 = make_event("$name2", et::NAME, "", "@bob:b.com", 2000);

        let mut set1 = HashMap::new();
        set1.insert(("m.room.name".to_string(), String::new()), name1);

        let mut set2 = HashMap::new();
        set2.insert(("m.room.name".to_string(), String::new()), name2);

        let resolved = resolve_state(&[set1, set2], &HashMap::new());
        assert_eq!(resolved.len(), 1);
        // Earlier timestamp (1000) wins when power levels are equal
        assert_eq!(
            resolved[&("m.room.name".to_string(), String::new())].event_id,
            "$name1"
        );
    }

    #[test]
    fn test_resolve_conflicted_higher_power_wins() {
        let power_levels = Pdu {
            event_id: "$pl".to_string(),
            room_id: "!room:example.com".to_string(),
            sender: "@admin:a.com".to_string(),
            event_type: "m.room.power_levels".to_string(),
            state_key: Some(String::new()),
            content: serde_json::json!({
                "users": {"@admin:a.com": 100, "@user:b.com": 0},
                "state_default": 50,
                "events_default": 0,
            }),
            origin_server_ts: 500,
            unsigned: None,
            stream_position: 0,
            origin: None,
            auth_events: None,
            prev_events: None,
            depth: None,
            hashes: None,
            signatures: None,
        };

        // Admin sets name at ts=2000, user sets name at ts=1000
        let name_admin = make_event("$name_admin", et::NAME, "", "@admin:a.com", 2000);
        let name_user = make_event("$name_user", et::NAME, "", "@user:b.com", 1000);

        let mut set1 = HashMap::new();
        set1.insert(
            ("m.room.power_levels".to_string(), String::new()),
            power_levels.clone(),
        );
        set1.insert(("m.room.name".to_string(), String::new()), name_admin);

        let mut set2 = HashMap::new();
        set2.insert(
            ("m.room.power_levels".to_string(), String::new()),
            power_levels,
        );
        set2.insert(("m.room.name".to_string(), String::new()), name_user);

        let resolved = resolve_state(&[set1, set2], &HashMap::new());
        // Admin (power 100) wins over user (power 0) despite later timestamp
        assert_eq!(
            resolved[&("m.room.name".to_string(), String::new())].event_id,
            "$name_admin"
        );
    }

    #[test]
    fn test_resolve_empty() {
        let resolved = resolve_state(&[], &HashMap::new());
        assert!(resolved.is_empty());
    }
}
