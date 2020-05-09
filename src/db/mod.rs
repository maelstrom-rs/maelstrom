pub mod mock;
pub mod postgres;

pub use postgres::PostgresStore;

use std;
use std::borrow::Cow;
use std::fmt;

use async_trait::async_trait;
use ruma_identifiers::{DeviceId, UserId};

use crate::models::auth::{PWHash, UserIdentifier};

/// A Storage Driver.
///
/// This trait encapsulates a complete storage driver to a
/// specific type of storage mechanism, e.g. Postgres, Kafka, etc.
#[async_trait]
pub trait Store: Clone + Sync + Send + Sized {
    /// Gets the type of this data store, e.g. Postgres
    fn get_type(&self) -> String;

    /// checks if username exists
    async fn check_username_exists(&self, username: &str) -> Result<bool, Error>;

    async fn check_device_id_exists(&self, device_id: &DeviceId) -> Result<bool, Error>;

    async fn remove_device_id(&self, device_id: &DeviceId, user_id: &UserId) -> Result<(), Error>;

    async fn remove_all_device_ids(&self, user_id: &UserId) -> Result<(), Error>;

    async fn fetch_user_id<'a>(
        &self,
        user_id: &'a UserIdentifier,
    ) -> Result<Option<Cow<'a, UserId>>, Error>;

    async fn fetch_password_hash(&self, user_id: &UserId) -> Result<PWHash, Error>;

    async fn check_otp_exists(&self, user_id: &UserId, otp: &str) -> Result<bool, Error>;

    async fn set_device<'a>(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        display_name: Option<&str>,
    ) -> Result<(), Error>;
}

/// Store Error
///
/// Errors returned from implementaitons of the `Store`
/// trait.
#[derive(Debug, Clone)]
pub struct Error {
    // TODO: Implement source error
    pub code: ErrorCode,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            // TODO: implment source error and chain dispaly
            ErrorCode::ConnectionFailed => write!(f, "Connection failed."),
            ErrorCode::AuthFailed => write!(f, "Authentication failed."),
            ErrorCode::RecordNotFound => write!(f, "The data store could not find any records."),
            ErrorCode::DuplicateViolation => {
                write!(f, "A Key or Unique constraint has been violated.")
            }
            ErrorCode::NullViolation => write!(f, "A non-Null constraint was violated."),
            ErrorCode::InvalidSyntax => write!(f, "The query contained invalid syntax."),
            _ => write!(f, "An unknown error has occurred."),
        }
    }
}

/// A generic list of error codes returned from storge/db servers.
///
/// Used to simplify error handling responsed from different storage
/// servers.  The actual implementation of the `Store` trait should use
/// additional logging to monitor for more granular and specific errors.
#[derive(Debug, Clone)]
pub enum ErrorCode {
    /// Cant connect to the storage/db server.
    ConnectionFailed,
    /// Authorization credentials failed.
    AuthFailed,
    /// Row/Record not found.
    RecordNotFound,
    /// A Unique Key or Duplicate Constaint check was violated.
    DuplicateViolation,
    /// A non-null check was violated.
    NullViolation,
    /// The query syntax was invalid.
    InvalidSyntax,
    /// Catch all for any error that can be translated to one of the existing errors.
    Unknown(String),
}
