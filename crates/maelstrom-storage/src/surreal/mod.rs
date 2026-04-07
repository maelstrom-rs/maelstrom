pub mod connection;
pub mod schema;

use async_trait::async_trait;
use maelstrom_core::identifiers::{DeviceId, UserId};
use surrealdb::Surreal;
use surrealdb::engine::any::Any;

use crate::traits::*;

/// SurrealDB-backed storage implementation.
///
/// This is the primary storage backend for Maelstrom.
/// It wraps a SurrealDB client and implements all storage traits.
#[derive(Clone)]
pub struct SurrealStorage {
    db: Surreal<Any>,
}

impl SurrealStorage {
    /// Create a new SurrealStorage from an existing connection.
    pub fn new(db: Surreal<Any>) -> Self {
        Self { db }
    }

    /// Get a reference to the underlying SurrealDB client.
    pub fn db(&self) -> &Surreal<Any> {
        &self.db
    }
}

#[async_trait]
impl UserStore for SurrealStorage {
    async fn create_user(&self, _user: &UserRecord) -> StorageResult<()> {
        todo!("Phase 2: User registration")
    }

    async fn get_user(&self, _localpart: &str) -> StorageResult<UserRecord> {
        todo!("Phase 2: User lookup")
    }

    async fn user_exists(&self, _localpart: &str) -> StorageResult<bool> {
        todo!("Phase 2: User existence check")
    }

    async fn set_password_hash(&self, _localpart: &str, _hash: &str) -> StorageResult<()> {
        todo!("Phase 2: Password update")
    }

    async fn set_deactivated(&self, _localpart: &str, _deactivated: bool) -> StorageResult<()> {
        todo!("Phase 2: Account deactivation")
    }

    async fn get_profile(&self, _localpart: &str) -> StorageResult<ProfileRecord> {
        todo!("Phase 2: Profile lookup")
    }

    async fn set_display_name(&self, _localpart: &str, _name: Option<&str>) -> StorageResult<()> {
        todo!("Phase 2: Display name update")
    }

    async fn set_avatar_url(&self, _localpart: &str, _url: Option<&str>) -> StorageResult<()> {
        todo!("Phase 2: Avatar URL update")
    }
}

#[async_trait]
impl DeviceStore for SurrealStorage {
    async fn create_device(&self, _device: &DeviceRecord) -> StorageResult<()> {
        todo!("Phase 2: Device creation")
    }

    async fn get_device(&self, _user_id: &UserId, _device_id: &DeviceId) -> StorageResult<DeviceRecord> {
        todo!("Phase 2: Device lookup")
    }

    async fn get_device_by_token(&self, _access_token: &str) -> StorageResult<DeviceRecord> {
        todo!("Phase 2: Token lookup")
    }

    async fn list_devices(&self, _user_id: &UserId) -> StorageResult<Vec<DeviceRecord>> {
        todo!("Phase 2: Device listing")
    }

    async fn remove_device(&self, _user_id: &UserId, _device_id: &DeviceId) -> StorageResult<()> {
        todo!("Phase 2: Device removal")
    }

    async fn remove_all_devices(&self, _user_id: &UserId) -> StorageResult<()> {
        todo!("Phase 2: Remove all devices")
    }
}

#[async_trait]
impl HealthCheck for SurrealStorage {
    async fn is_healthy(&self) -> bool {
        self.db.health().await.is_ok()
    }
}
