pub mod auth;
pub mod json;

pub use auth::AuthenticatedUser;
pub use json::MatrixJson;

use maelstrom_core::error::{ErrorCode, MatrixError};
use maelstrom_media::client::MediaError;
use maelstrom_storage::traits::StorageError;

/// Convert StorageError to MatrixError with proper status codes and error discrimination.
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

/// Convert MediaError to MatrixError.
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
