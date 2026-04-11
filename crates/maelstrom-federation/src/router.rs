//! # Federation Router
//!
//! Assembles all server-to-server (federation) endpoints into a single Axum [`Router`].
//!
//! The federation API lives under two URL prefixes:
//!
//! - **`/_matrix/federation/v1/`** and **`/_matrix/federation/v2/`** -- the main
//!   server-to-server API endpoints for sending transactions, joining rooms, querying
//!   state, and more.
//! - **`/_matrix/key/v2/`** -- the key distribution API where servers publish and
//!   query Ed25519 signing keys.
//!
//! ## Endpoint Groups
//!
//! The router merges sub-routers from each module:
//!
//! | Module          | Key Endpoints                                           |
//! |-----------------|---------------------------------------------------------|
//! | [`key_server`]  | `GET /key/v2/server`, `GET /key/v2/query/{server}`      |
//! | [`receiver`]    | `PUT /federation/v1/send/{txnId}`                       |
//! | [`joins`]       | `GET make_join`, `PUT send_join`, `GET make_leave`, etc.|
//! | [`state`]       | `GET /state/{roomId}`, `GET /state_ids/{roomId}`, `GET /event/{eventId}` |
//! | [`backfill`]    | `GET /backfill/{roomId}`, `POST /get_missing_events/{roomId}` |
//! | [`user_keys`]   | `POST /user/keys/query`                                 |
//! | [`queries`]     | `GET /query/profile`, `GET /query/directory`, `GET /publicRooms`, `POST /publicRooms` |
//! | [`invite`]      | `PUT /invite/{roomId}/{eventId}` (v1 and v2)            |
//!
//! [`key_server`]: crate::key_server
//! [`receiver`]: crate::receiver
//! [`joins`]: crate::joins
//! [`state`]: crate::state
//! [`backfill`]: crate::backfill
//! [`user_keys`]: crate::user_keys
//! [`queries`]: crate::queries
//! [`invite`]: crate::invite

use axum::Router;

use crate::FederationState;

/// Build the complete federation router with all server-to-server endpoints.
///
/// All routes share the provided [`FederationState`] via Axum's state extraction.
/// The returned router should be merged into the main application router,
/// typically alongside the client-server API router.
pub fn build(state: FederationState) -> Router {
    let federation_api = Router::new()
        .merge(crate::key_server::routes())
        .merge(crate::receiver::routes())
        .merge(crate::joins::routes())
        .merge(crate::state::routes())
        .merge(crate::backfill::routes())
        .merge(crate::user_keys::routes())
        .merge(crate::queries::routes())
        .merge(crate::invite::routes());

    Router::new().merge(federation_api).with_state(state)
}
