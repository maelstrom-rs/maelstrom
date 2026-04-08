use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::error::MatrixError;

use crate::AdminState;
use crate::auth::AdminUser;

pub fn routes() -> Router<AdminState> {
    Router::new()
        .route("/_maelstrom/admin/v1/rooms", get(list_rooms))
        .route("/_maelstrom/admin/v1/rooms/{roomId}", get(get_room))
        .route(
            "/_maelstrom/admin/v1/rooms/{roomId}/shutdown",
            post(shutdown_room),
        )
}

#[derive(Deserialize)]
struct ListRoomsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    from: Option<String>,
}

fn default_limit() -> usize {
    100
}

async fn list_rooms(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Query(query): Query<ListRoomsQuery>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let (rooms, total) = state
        .storage()
        .get_public_rooms(query.limit, query.from.as_deref(), None)
        .await
        .map_err(|e| MatrixError::unknown(format!("{e}")))?;

    let room_list: Vec<serde_json::Value> = rooms
        .iter()
        .map(|r| {
            serde_json::json!({
                "room_id": r.room_id,
                "name": r.name,
                "topic": r.topic,
                "num_joined_members": r.num_joined_members,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "rooms": room_list,
        "total": total,
    })))
}

async fn get_room(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(room_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let room = state
        .storage()
        .get_room(&room_id)
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let members = state
        .storage()
        .get_room_members(&room_id, "join")
        .await
        .unwrap_or_default();

    let state_events = state
        .storage()
        .get_current_state(&room_id)
        .await
        .unwrap_or_default();

    let name = state_events
        .iter()
        .find(|e| e.event_type == "m.room.name")
        .and_then(|e| e.content.get("name").and_then(|n| n.as_str()));

    let topic = state_events
        .iter()
        .find(|e| e.event_type == "m.room.topic")
        .and_then(|e| e.content.get("topic").and_then(|t| t.as_str()));

    Ok(Json(serde_json::json!({
        "room_id": room.room_id,
        "version": room.version,
        "creator": room.creator,
        "is_direct": room.is_direct,
        "name": name,
        "topic": topic,
        "num_joined_members": members.len(),
        "members": members,
        "state_event_count": state_events.len(),
    })))
}

async fn shutdown_room(
    State(state): State<AdminState>,
    _admin: AdminUser,
    Path(room_id): Path<String>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    // Get all joined members and set them to "leave"
    let members = state
        .storage()
        .get_room_members(&room_id, "join")
        .await
        .map_err(|_| MatrixError::not_found("Room not found"))?;

    let mut kicked = 0;
    for member in &members {
        if state
            .storage()
            .set_membership(member, &room_id, "leave")
            .await
            .is_ok()
        {
            kicked += 1;
        }
    }

    Ok(Json(serde_json::json!({
        "status": "shutdown",
        "room_id": room_id,
        "kicked_users": kicked,
    })))
}
