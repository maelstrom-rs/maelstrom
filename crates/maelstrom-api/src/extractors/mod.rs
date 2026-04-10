//! Axum extractors for Matrix authentication and JSON body parsing.
//!
//! Extractors are Axum's dependency-injection mechanism.  When you put a type
//! that implements `FromRequestParts` or `FromRequest` in a handler's parameter
//! list, Axum runs its extraction logic *before* your handler is called.  If
//! extraction fails, the handler is never invoked and the extractor's `Rejection`
//! type is returned to the client instead.
//!
//! This module provides two extractors that most handlers use:
//!
//! - [`AuthenticatedUser`] -- validates the access token and provides the
//!   caller's `user_id` and `device_id`.  Any handler that includes this in
//!   its parameter list automatically requires authentication.
//! - [`MatrixJson`] -- parses a JSON request body and returns Matrix-spec
//!   error codes (`M_NOT_JSON`, `M_BAD_JSON`) on failure, instead of Axum's
//!   default plain-text error.
//!
//! It also exposes helper functions ([`storage_error`] and [`media_error`])
//! that map backend errors to Matrix-spec HTTP error responses.

pub mod auth;
pub mod json;

pub use auth::AuthenticatedUser;
pub use json::MatrixJson;

use maelstrom_core::matrix::error::{ErrorCode, MatrixError};
use maelstrom_media::client::MediaError;
use maelstrom_storage::traits::StorageError;

/// Convert a [`StorageError`] into a [`MatrixError`] with the appropriate HTTP
/// status code and Matrix error code.
///
/// The mapping:
/// - `NotFound` -> 404 with the standard "Not found" message.
/// - `Duplicate` -> 409 Conflict (e.g. trying to create a user that already exists).
/// - `Connection`, `Query`, `Serialization`, `Internal` -> 500 with a generic
///   "Internal server error" message.  The real details are logged server-side
///   via `tracing::error!` so they never leak to the client.
pub fn storage_error(e: StorageError) -> MatrixError {
    match e {
        StorageError::NotFound => MatrixError::not_found("Not found"),
        StorageError::Duplicate(msg) => MatrixError::new(
            http::StatusCode::CONFLICT,
            ErrorCode::Unknown,
            format!("Duplicate: {msg}"),
        ),
        StorageError::Connection(msg) => {
            tracing::error!("Storage connection error: {msg}");
            MatrixError::unknown("Internal server error")
        }
        StorageError::Query(msg) => {
            tracing::error!("Storage query error: {msg}");
            MatrixError::unknown("Internal server error")
        }
        StorageError::Serialization(msg) => {
            tracing::error!("Storage serialization error: {msg}");
            MatrixError::unknown("Internal server error")
        }
        StorageError::Internal(msg) => {
            tracing::error!("Storage internal error: {msg}");
            MatrixError::unknown("Internal server error")
        }
    }
}

/// Convert a [`MediaError`] into a [`MatrixError`] with the appropriate HTTP
/// status code and Matrix error code.
///
/// - `NotFound` -> 404 (the media ID doesn't exist in the store).
/// - `Upload` / `Download` / `Connection` -> 500 with a generic message.
///   Internal details are logged but never exposed to the client.
pub fn media_error(e: MediaError) -> MatrixError {
    match e {
        MediaError::NotFound(msg) => MatrixError::not_found(&msg),
        MediaError::Upload(msg) => {
            tracing::error!("Media upload error: {msg}");
            MatrixError::unknown("Media upload failed")
        }
        MediaError::Download(msg) => {
            tracing::error!("Media download error: {msg}");
            MatrixError::unknown("Media download failed")
        }
        MediaError::Connection(msg) => {
            tracing::error!("Media connection error: {msg}");
            MatrixError::unknown("Internal server error")
        }
    }
}
