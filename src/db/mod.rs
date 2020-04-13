pub mod postgres;

pub use postgres::PostgresStore;

use async_trait::async_trait;
use std::error::Error;

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
}
