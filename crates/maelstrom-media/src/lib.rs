//! Media processing stack for Maelstrom.
//!
//! This crate handles all media operations required by the Matrix Content
//! Repository API (`/_matrix/media/`):
//!
//! - **Storage** ([`client::MediaClient`]) -- Wraps the AWS S3 SDK to talk to any
//!   S3-compatible object store (RustFS in production, MinIO for local dev).
//!   Media objects are keyed by `{server_name}/{media_id}` inside a single bucket,
//!   matching the MXC URI scheme (`mxc://server/media_id`).
//!
//! - **Thumbnails** ([`thumbnail`]) -- On-the-fly thumbnail generation using the
//!   `image` crate. Supports `scale` (fit within bounds, preserve aspect ratio)
//!   and `crop` (fill exact dimensions) resize methods. Input formats: PNG, JPEG,
//!   GIF, WebP. Output is always PNG.
//!
//! - **URL Previews** ([`preview`]) -- Fetches an arbitrary URL over HTTP, parses
//!   the HTML for OpenGraph `<meta>` tags (`og:title`, `og:image`, `og:description`,
//!   etc.), and returns structured metadata for the `/_matrix/media/v3/preview_url`
//!   endpoint. Non-HTML responses and fetch failures return empty metadata rather
//!   than errors (the Matrix spec treats previews as best-effort).
//!
//! - **Retention** ([`retention`]) -- A background Tokio task that periodically
//!   sweeps for media older than a configurable `max_age_days`, deleting both the
//!   S3 object and the database metadata record in batches.

pub mod client;
pub mod preview;
pub mod retention;
pub mod thumbnail;
