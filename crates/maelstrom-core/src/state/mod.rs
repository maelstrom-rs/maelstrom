//! State Resolution v2 algorithm.
//!
//! Implements the state resolution algorithm as specified in:
//! https://spec.matrix.org/latest/rooms/v2/
//!
//! Given conflicting sets of state events (from different branches of the DAG),
//! determines the resolved state.

use std::collections::{HashMap, HashSet};

use crate::events::pdu::StoredEvent;

/// A state key tuple: (event_type, state_key).
type StateKey = (String, String);

/// Resolve conflicting room state using the v2 algorithm.
///
/// `state_sets` is a list of state maps, each from a different branch of the DAG.
/// Each state map is `(event_type, state_key) -> StoredEvent`.
///
/// Returns the resolved state map.
pub fn resolve_state(
    state_sets: &[HashMap<StateKey, StoredEvent>],
    _auth_events: &HashMap<String, StoredEvent>,
) -> HashMap<StateKey, StoredEvent> {
    if state_sets.is_empty() {
        return HashMap::new();
    }

    if state_sets.len() == 1 {
        return state_sets[0].clone();
    }

    // Step 1: Compute the unconflicted state (events present in all sets with the same event_id).
    let mut unconflicted = HashMap::new();
    let mut conflicted: HashMap<StateKey, Vec<StoredEvent>> = HashMap::new();

    // Collect all state keys across all sets.
    let mut all_keys: HashSet<StateKey> = HashSet::new();
    for set in state_sets {
        for key in set.keys() {
            all_keys.insert(key.clone());
        }
    }

    for key in &all_keys {
        let mut event_ids: Vec<Option<&str>> = Vec::new();
        let mut events: Vec<&StoredEvent> = Vec::new();

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
            let unique_events: Vec<StoredEvent> =
                events.iter().map(|e| (*e).clone()).collect::<Vec<_>>();
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

/// Get the power level of a sender from the power_levels event content.
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
fn is_auth_allowed(event: &StoredEvent, current_state: &HashMap<StateKey, StoredEvent>) -> bool {
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
    use super::*;

    fn make_event(
        event_id: &str,
        event_type: &str,
        state_key: &str,
        sender: &str,
        ts: u64,
    ) -> StoredEvent {
        StoredEvent {
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
            make_event("$create", "m.room.create", "", "@alice:a.com", 1000),
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
        let create = make_event("$create", "m.room.create", "", "@alice:a.com", 1000);
        let name = make_event("$name", "m.room.name", "", "@alice:a.com", 2000);

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
        let name1 = make_event("$name1", "m.room.name", "", "@alice:a.com", 1000);
        let name2 = make_event("$name2", "m.room.name", "", "@bob:b.com", 2000);

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
        let power_levels = StoredEvent {
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
        let name_admin = make_event("$name_admin", "m.room.name", "", "@admin:a.com", 2000);
        let name_user = make_event("$name_user", "m.room.name", "", "@user:b.com", 1000);

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
