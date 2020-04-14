use std::borrow::Cow;
use std::error::Error;

use async_trait::async_trait;
use ruma_identifiers::{DeviceId, UserId};
use sqlx::postgres::PgPool;
use sqlx::postgres::PgQueryAs;

use super::Store;
use crate::models::auth::{PWHash, UserIdentifier};

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

    async fn check_username_exists(&self, username: &str) -> Result<bool, Box<dyn Error>> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts where localpart = $1")
            .bind(username)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.0 > 0)
    }

    async fn fetch_user_id<'a>(
        &self,
        user_id: &'a UserIdentifier,
    ) -> Result<Option<Cow<'a, UserId>>, Box<dyn Error>> {
        unimplemented!()
    }

    async fn fetch_password_hash(&self, user_id: &UserId) -> Result<PWHash, Box<dyn Error>> {
        unimplemented!()
    }

    async fn check_otp_exists(&self, user_id: &UserId, otp: &str) -> Result<bool, Box<dyn Error>> {
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
