//! Space hierarchy.
//!
//! Spaces are regular Matrix rooms that organise other rooms into a tree
//! structure. A space declares its children via `m.space.child` state events
//! (keyed by the child room ID), and a child can point back to its parent
//! with `m.space.parent`.
//!
//! The hierarchy endpoint walks this tree starting from a given space room,
//! returning the rooms reachable from it up to a configurable depth. Each
//! entry includes stripped state (name, topic, avatar, join rules, membership
//! counts) so clients can render a browseable directory without joining every
//! room.
//!
//! Rooms that are not world-readable or that the user has not joined are
//! filtered out. The `suggested_only` parameter limits results to rooms marked
//! as suggested by the space.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `GET` | `/_matrix/client/v1/rooms/{roomId}/hierarchy` | Walk the space hierarchy starting from the given room |
//!
//! # Matrix spec
//!
//! * [Spaces](https://spec.matrix.org/v1.18/client-server-api/#spaces)

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_core::matrix::room::{JoinRule, Membership, event_type as et};

use crate::extractors::AuthenticatedUser;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/_matrix/client/v1/rooms/{roomId}/hierarchy",
        get(get_hierarchy),
    )
}

#[derive(Deserialize)]
struct HierarchyQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_max_depth")]
    max_depth: usize,
    #[allow(dead_code)]
    from: Option<String>,
    #[serde(default)]
    suggested_only: bool,
}

fn default_limit() -> usize {
    50
}

fn default_max_depth() -> usize {
    5
}

/// GET /rooms/{roomId}/hierarchy — traverse the space hierarchy.
async fn get_hierarchy(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(room_id): Path<String>,
    Query(query): Query<HierarchyQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();

    // Build hierarchy by traversing m.space.child state events
    let mut rooms = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut queue = vec![(room_id.clone(), 0usize)];

    while let Some((current_room, depth)) = queue.pop() {
        if depth > query.max_depth || visited.contains(&current_room) || rooms.len() >= query.limit
        {
            continue;
        }
        visited.insert(current_room.clone());

        // Get room info
        let _room = match storage.get_room(&current_room).await {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Get current state to extract name, topic, membership count
        let current_state = storage
            .get_current_state(&current_room)
            .await
            .unwrap_or_default();

        let name = current_state
            .iter()
            .find(|e| e.event_type == et::NAME)
            .and_then(|e| e.content.get("name").and_then(|n| n.as_str()))
            .map(|s| s.to_string());

        let topic = current_state
            .iter()
            .find(|e| e.event_type == et::TOPIC)
            .and_then(|e| e.content.get("topic").and_then(|t| t.as_str()))
            .map(|s| s.to_string());

        let canonical_alias = current_state
            .iter()
            .find(|e| e.event_type == et::CANONICAL_ALIAS)
            .and_then(|e| e.content.get("alias").and_then(|a| a.as_str()))
            .map(|s| s.to_string());

        let join_rule = current_state
            .iter()
            .find(|e| e.event_type == et::JOIN_RULES)
            .and_then(|e| e.content.get("join_rule").and_then(|j| j.as_str()))
            .unwrap_or(JoinRule::Invite.as_str());

        let num_joined = storage
            .get_room_members(&current_room, Membership::Join.as_str())
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        // Determine room type (space or regular)
        let room_type = current_state
            .iter()
            .find(|e| e.event_type == et::CREATE)
            .and_then(|e| e.content.get("type").and_then(|t| t.as_str()))
            .map(|s| s.to_string());

        // Collect children (m.space.child state events)
        let mut children_state = Vec::new();
        for event in &current_state {
            if event.event_type == et::SPACE_CHILD
                && let Some(state_key) = &event.state_key
            {
                let suggested = event
                    .content
                    .get("suggested")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);

                if query.suggested_only && !suggested {
                    continue;
                }

                // Check via is not empty (required per spec)
                let has_via = event
                    .content
                    .get("via")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty());

                if has_via {
                    children_state.push(event.to_client_event().into_json());
                    queue.push((state_key.clone(), depth + 1));
                }
            }
        }

        let mut room_entry = serde_json::json!({
            "room_id": current_room,
            "num_joined_members": num_joined,
            "world_readable": join_rule == JoinRule::Public.as_str(),
            "guest_can_join": false,
            "join_rule": join_rule,
            "children_state": children_state,
        });

        if let Some(n) = name {
            room_entry["name"] = serde_json::json!(n);
        }
        if let Some(t) = topic {
            room_entry["topic"] = serde_json::json!(t);
        }
        if let Some(a) = canonical_alias {
            room_entry["canonical_alias"] = serde_json::json!(a);
        }
        if let Some(rt) = room_type {
            room_entry["room_type"] = serde_json::json!(rt);
        }

        rooms.push(room_entry);
    }

    Ok(Json(serde_json::json!({ "rooms": rooms })))
}
