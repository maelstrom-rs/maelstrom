//! Handler modules for the Matrix Client-Server API.
//!
//! Each sub-module maps to a section of the
//! [Matrix Client-Server API specification](https://spec.matrix.org/latest/client-server-api/):
//!
//! | Module | Spec section |
//! |---|---|
//! | [`register`] | Account registration (including guest access and UIA) |
//! | [`auth`] | Login / logout / token refresh |
//! | [`profile`] | Display name, avatar URL |
//! | [`account`] | Account data, deactivation, whoami |
//! | [`rooms`] | Room creation, joining, leaving, state, sending events |
//! | [`sync`] | The `/sync` long-poll endpoint -- the heart of the client API |
//! | [`events`] | Fetching individual events and room context |
//! | [`directory`] | Room directory (public room lists, room aliases) |
//! | [`keys`] | End-to-end encryption key uploads, queries, and claims |
//! | [`to_device`] | Device-to-device messaging (key sharing, verification) |
//! | [`typing`] | Typing indicators |
//! | [`receipts`] | Read receipts |
//! | [`presence`] | Online/offline/unavailable status |
//! | [`media`] | File upload, download, and thumbnails |
//! | [`search`] | Full-text message search |
//! | [`spaces`] | Space hierarchy (MSC2946) |
//! | [`relations`] | Event relationships (threads, replies, annotations) |
//! | [`knock`] | Knock-to-join |
//! | [`reporting`] | Reporting abusive content |
//! | [`threads`] | Thread listing |
//! | [`capabilities`] | Server capability advertisement |
//! | [`versions`] | Supported spec versions |
//! | [`wellknown`] | `.well-known` server/client discovery |
//! | [`health`] | Health check endpoint |
//!
//! ## Convention
//!
//! Every module exposes a `routes()` function that returns a `Router<AppState>`.
//! The [`crate::router::build`] function merges them all into the final router.
//! Handler functions are `async fn`s that take Axum extractors as parameters
//! and return `Result<Json<...>, MatrixError>`.

pub mod account;
pub mod auth;
pub mod capabilities;
pub mod directory;
pub mod events;
pub mod health;
pub mod keys;
pub mod knock;
pub mod media;
pub mod presence;
pub mod profile;
pub mod receipts;
pub mod register;
pub mod relations;
pub mod reporting;
pub mod rooms;
pub mod search;
pub mod spaces;
pub mod sync;
pub mod threads;
pub mod to_device;
pub mod typing;
pub mod util;
pub mod versions;
pub mod wellknown;
