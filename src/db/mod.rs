pub mod postgres;

pub use postgres::PostgresStore;

use std::borrow::Cow;
use std::error::Error;

use async_trait::async_trait;
use ruma_identifiers::{DeviceId, UserId};

use crate::models::auth::UserIdentifier;

/// A Storage Driver.
///
/// This trait encapsulates a complete storage driver to a
/// specific type of storage mechanism, e.g. Postgres, Kafka, etc.
#[async_trait]
pub trait Store: Clone + Sync + Send + Sized {
    /// Gets the type of this data store, e.g. Postgres
    fn get_type(&self) -> String;

    /// Determines if a username is available for registration.
    /// TODO: Create more generic error responses
    async fn is_username_available(&self, username: &str) -> Result<bool, Box<dyn Error>>;

    async fn check_password<'a>(
        &self,
        user_id: &'a UserIdentifier,
        password: &str,
    ) -> Result<Option<Cow<'a, UserId>>, Box<dyn Error>>;

    async fn check_otp<'a>(
        &self,
        user_id: &'a UserIdentifier,
        otp: &str,
    ) -> Result<Option<Cow<'a, UserId>>, Box<dyn Error>>;

    async fn set_device<'a>(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        display_name: Option<&str>,
    ) -> Result<(), Box<dyn Error>>;
}
