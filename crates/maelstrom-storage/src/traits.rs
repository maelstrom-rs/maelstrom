use async_trait::async_trait;
use maelstrom_core::identifiers::{DeviceId, UserId};
use serde::{Deserialize, Serialize};

/// Result type for storage operations.
pub type StorageResult<T> = Result<T, StorageError>;

/// Errors that can occur during storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Record not found")]
    NotFound,

    #[error("Duplicate record: {0}")]
    Duplicate(String),

    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Query failed: {0}")]
    Query(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// A stored device record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub user_id: String,
    pub display_name: Option<String>,
    pub access_token: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A stored user record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub localpart: String,
    pub password_hash: Option<String>,
    pub is_admin: bool,
    pub is_guest: bool,
    pub is_deactivated: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A user profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRecord {
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

/// User account storage operations.
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn create_user(&self, user: &UserRecord) -> StorageResult<()>;
    async fn get_user(&self, localpart: &str) -> StorageResult<UserRecord>;
    async fn user_exists(&self, localpart: &str) -> StorageResult<bool>;
    async fn set_password_hash(&self, localpart: &str, hash: &str) -> StorageResult<()>;
    async fn set_deactivated(&self, localpart: &str, deactivated: bool) -> StorageResult<()>;
    async fn get_profile(&self, localpart: &str) -> StorageResult<ProfileRecord>;
    async fn set_display_name(&self, localpart: &str, name: Option<&str>) -> StorageResult<()>;
    async fn set_avatar_url(&self, localpart: &str, url: Option<&str>) -> StorageResult<()>;
}

/// Device and access token storage operations.
#[async_trait]
pub trait DeviceStore: Send + Sync {
    async fn create_device(&self, device: &DeviceRecord) -> StorageResult<()>;
    async fn get_device(&self, user_id: &UserId, device_id: &DeviceId) -> StorageResult<DeviceRecord>;
    async fn get_device_by_token(&self, access_token: &str) -> StorageResult<DeviceRecord>;
    async fn list_devices(&self, user_id: &UserId) -> StorageResult<Vec<DeviceRecord>>;
    async fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) -> StorageResult<()>;
    async fn remove_all_devices(&self, user_id: &UserId) -> StorageResult<()>;
}

/// Health check for storage backends.
#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn is_healthy(&self) -> bool;
}

/// Combined storage trait for the complete storage backend.
/// Individual stores are split for clarity but a single backend implements all.
pub trait Storage: UserStore + DeviceStore + HealthCheck + Send + Sync + 'static {}

/// Blanket implementation: anything that implements all sub-traits is a Storage.
impl<T> Storage for T where T: UserStore + DeviceStore + HealthCheck + Send + Sync + 'static {}
