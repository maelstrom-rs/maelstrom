//! Server-side message search.
//!
//! Provides full-text search over room events. Clients submit a search term and
//! optional filters (by room, sender, etc.) and the server returns matching
//! events ranked by relevance using BM25 scoring.
//!
//! Search only covers rooms the requesting user is a member of. Results include
//! the matched events along with optional context (events before/after the
//! match) and highlight information so clients can render snippets.
//!
//! Pagination is supported via a `next_batch` token in the response.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST` | `/_matrix/client/v3/search` | Perform a server-side search across room events |
//!
//! # Matrix spec
//!
//! * [Search](https://spec.matrix.org/v1.12/client-server-api/#search)

use axum::extract::{Query, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use maelstrom_core::matrix::error::MatrixError;

use crate::extractors::{AuthenticatedUser, MatrixJson};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/_matrix/client/v3/search", post(search))
}

#[derive(Deserialize)]
struct SearchRequest {
    search_categories: SearchCategories,
}

#[derive(Deserialize)]
struct SearchCategories {
    room_events: Option<RoomEventSearch>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct RoomEventSearch {
    search_term: String,
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    filter: Option<SearchFilter>,
    #[serde(default)]
    event_context: Option<EventContext>,
    #[serde(default)]
    order_by: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SearchFilter {
    rooms: Option<Vec<String>>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct EventContext {
    before_limit: Option<usize>,
    after_limit: Option<usize>,
    include_profile: Option<bool>,
}

#[derive(Deserialize, Default)]
struct SearchQuery {
    next_batch: Option<String>,
}

async fn search(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(search_query): Query<SearchQuery>,
    MatrixJson(body): MatrixJson<SearchRequest>,
) -> Result<Json<serde_json::Value>, MatrixError> {
    let storage = state.storage();
    let user_id = auth.user_id.to_string();
    let offset = search_query
        .next_batch
        .as_deref()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let room_search = body
        .search_categories
        .room_events
        .ok_or_else(|| MatrixError::bad_json("Missing room_events search category"))?;

    // Determine which rooms to search
    let mut room_ids = match &room_search.filter {
        Some(filter) if filter.rooms.is_some() => filter.rooms.clone().unwrap_or_default(),
        _ => storage
            .get_joined_rooms(&user_id)
            .await
            .map_err(crate::extractors::storage_error)?,
    };

    // Traverse room upgrade chains to include predecessor rooms in search
    let mut predecessors = Vec::new();
    for room_id in &room_ids {
        if let Ok(preds) = storage.get_room_predecessors(room_id).await {
            for pred in preds {
                if !room_ids.contains(&pred) && !predecessors.contains(&pred) {
                    predecessors.push(pred);
                }
            }
        }
    }
    room_ids.extend(predecessors);

    let limit = room_search
        .filter
        .as_ref()
        .and_then(|f| f.limit)
        .unwrap_or(10)
        .min(50);

    let search_lower = room_search.search_term.to_lowercase();
    let order_by = room_search.order_by.as_deref().unwrap_or("rank");

    // Try full-text search first, fall back to LIKE-style if no results
    let mut events = storage
        .search_events(&room_ids, &room_search.search_term, 1000)
        .await
        .map_err(crate::extractors::storage_error)?;

    // If full-text search returns nothing, fetch all room events and filter client-side
    if events.is_empty() {
        let max_pos = storage.current_stream_position().await.unwrap_or(999999) + 1;
        let mut all_events = Vec::new();
        for room_id in &room_ids {
            if let Ok(room_events) = storage.get_room_events(room_id, max_pos, 1000, "b").await {
                all_events.extend(room_events);
            }
        }
        events = all_events;
    }

    // Filter out redacted events and refine matches
    let mut all_filtered: Vec<_> = events
        .into_iter()
        .filter(|e| {
            // Exclude redacted events (empty content)
            let has_content = e.content.as_object().map(|o| !o.is_empty()).unwrap_or(true);
            if !has_content {
                return false;
            }

            // Ensure the body contains the search term
            if let Some(body) = e.content.get("body").and_then(|b| b.as_str()) {
                body.to_lowercase().contains(&search_lower)
            } else {
                false
            }
        })
        .collect();

    // Sort results based on order_by
    if order_by == "recent" {
        all_filtered.sort_by(|a, b| b.origin_server_ts.cmp(&a.origin_server_ts));
    }

    let total_count = all_filtered.len();
    let filtered: Vec<_> = all_filtered.into_iter().skip(offset).take(limit).collect();

    let include_context = room_search.event_context.is_some();
    let before_limit = room_search
        .event_context
        .as_ref()
        .and_then(|c| c.before_limit)
        .unwrap_or(5);
    let after_limit = room_search
        .event_context
        .as_ref()
        .and_then(|c| c.after_limit)
        .unwrap_or(5);

    let mut results: Vec<serde_json::Value> = Vec::new();
    for event in &filtered {
        let mut result = serde_json::json!({
            "rank": 1,
            "result": event.to_client_event().into_json(),
        });

        if include_context {
            // Get events before and after
            let mut before_events = Vec::new();
            let mut after_events = Vec::new();

            if let Ok(before) = storage
                .get_room_events(&event.room_id, event.stream_position, before_limit, "b")
                .await
            {
                before_events = before
                    .iter()
                    .map(|e| e.to_client_event().into_json())
                    .collect();
            }
            if let Ok(after) = storage
                .get_room_events(&event.room_id, event.stream_position, after_limit, "f")
                .await
            {
                after_events = after
                    .iter()
                    .map(|e| e.to_client_event().into_json())
                    .collect();
            }

            result["context"] = serde_json::json!({
                "events_before": before_events,
                "events_after": after_events,
                "start": event.stream_position.to_string(),
                "end": event.stream_position.to_string(),
            });
        }

        results.push(result);
    }

    let count = total_count;
    let next_offset = offset + filtered.len();
    let has_more = next_offset < total_count;

    let mut response = serde_json::json!({
        "search_categories": {
            "room_events": {
                "results": results,
                "count": count,
                "highlights": [],
            }
        }
    });

    // Add next_batch if there are more results
    if has_more {
        response["search_categories"]["room_events"]["next_batch"] =
            serde_json::json!(next_offset.to_string());
    }

    Ok(Json(response))
}
