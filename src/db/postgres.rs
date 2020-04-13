use std::borrow::Cow;
use std::error::Error;

use async_trait::async_trait;
use ruma_identifiers::{DeviceId, UserId};
use sqlx::postgres::PgPool;
use sqlx::postgres::PgQueryAs;

use super::Store;
use crate::models::auth::UserIdentifier;

/// A Postgres Data Store
///
/// This implements the `Store` trait for Postgres.
#[derive(Clone)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    /// Returns a new PostgresStore from database connection url.
    pub async fn new(url: &str) -> Result<Self, sqlx::Error> {
        // TODO: Extract more config from env or such
        let pool = PgPool::builder()
            .max_size(5) // maximum number of connections in the pool
            .build(url)
            .await?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl Store for PostgresStore {
    fn get_type(&self) -> String {
        "Initialized PostgresStore".to_string()
    }

    async fn is_username_available(&self, username: &str) -> Result<bool, Box<dyn Error>> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts where localpart = $1")
            .bind(username)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.0 == 0)
    }

    async fn check_password<'a>(
        &self,
        user_id: &'a UserIdentifier,
        password: &str,
    ) -> Result<Option<Cow<'a, UserId>>, Box<dyn Error>> {
        unimplemented!()
    }

    async fn check_otp<'a>(
        &self,
        user_id: &'a UserIdentifier,
        otp: &str,
    ) -> Result<Option<Cow<'a, UserId>>, Box<dyn Error>> {
        unimplemented!()
    }

    async fn set_device<'a>(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        display_name: Option<&str>,
    ) -> Result<(), Box<dyn Error>> {
        unimplemented!()
    }
}
