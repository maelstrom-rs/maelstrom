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

    async fn fetch_user_id<'a>(
        &self,
        user_id: &'a UserIdentifier,
    ) -> Result<Option<Cow<'a, UserId>>, Box<dyn Error>>;

    async fn fetch_password_hash(&self, user_id: &UserId) -> Result<PWHash, Box<dyn Error>>;

    async fn check_otp(&self, user_id: &UserId, otp: &str) -> Result<bool, Box<dyn Error>>;

    async fn set_device<'a>(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        display_name: Option<&str>,
    ) -> Result<(), Box<dyn Error>>;
}

// TODO
pub enum PWHash {}
impl PWHash {
    pub fn matches(&self, pw: &str) -> bool {
        unimplemented!()
    }
}
