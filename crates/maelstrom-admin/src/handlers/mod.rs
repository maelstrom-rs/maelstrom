//! Admin API handler modules.
//!
//! Each sub-module groups related admin operations and exposes a `routes()`
//! function that returns a [`Router<AdminState>`] fragment. The parent
//! [`super::router`] merges all of these into the final admin router.
//!
//! - [`dashboard`] -- SSR HTML pages for the browser-based admin UI.
//! - [`users`]     -- user account CRUD, deactivation, admin promotion, password reset.
//! - [`rooms`]     -- room listing, inspection, and forced shutdown.
//! - [`media`]     -- per-user media listing, quarantine/unquarantine, retention config.
//! - [`federation`] -- federation signing-key statistics.
//! - [`server`]    -- server info, detailed health check, Prometheus metrics stub.
//! - [`reports`]   -- content report listing (placeholder for moderation queue).

pub mod dashboard;
pub mod federation;
pub mod media;
pub mod reports;
pub mod rooms;
pub mod server;
pub mod users;
