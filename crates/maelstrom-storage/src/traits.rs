use async_trait::async_trait;
use maelstrom_core::events::pdu::StoredEvent;
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

/// A room record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomRecord {
    pub room_id: String,
    pub version: String,
    pub creator: String,
    pub is_direct: bool,
}

/// A public room listing entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicRoom {
    pub room_id: String,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub canonical_alias: Option<String>,
    pub avatar_url: Option<String>,
    pub num_joined_members: usize,
    pub world_readable: bool,
    pub guest_can_join: bool,
}

/// User account storage operations.
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn create_user(&self, user: &UserRecord) -> StorageResult<()>;
    async fn get_user(&self, localpart: &str) -> StorageResult<UserRecord>;
    async fn user_exists(&self, localpart: &str) -> StorageResult<bool>;
    async fn set_password_hash(&self, localpart: &str, hash: &str) -> StorageResult<()>;
    async fn set_deactivated(&self, localpart: &str, deactivated: bool) -> StorageResult<()>;
    async fn set_admin(&self, localpart: &str, is_admin: bool) -> StorageResult<()>;
    async fn count_users(&self) -> StorageResult<u64>;
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
    async fn remove_all_devices_except(&self, user_id: &UserId, keep_device_id: &DeviceId) -> StorageResult<()>;
    async fn update_device_display_name(&self, user_id: &UserId, device_id: &DeviceId, display_name: Option<&str>) -> StorageResult<()>;
}

/// Room storage operations.
#[async_trait]
pub trait RoomStore: Send + Sync {
    async fn create_room(&self, room: &RoomRecord) -> StorageResult<()>;
    async fn get_room(&self, room_id: &str) -> StorageResult<RoomRecord>;
    async fn set_membership(&self, user_id: &str, room_id: &str, membership: &str) -> StorageResult<()>;
    async fn get_membership(&self, user_id: &str, room_id: &str) -> StorageResult<String>;
    async fn get_joined_rooms(&self, user_id: &str) -> StorageResult<Vec<String>>;
    async fn get_invited_rooms(&self, user_id: &str) -> StorageResult<Vec<String>>;
    async fn get_left_rooms(&self, user_id: &str) -> StorageResult<Vec<String>>;
    async fn get_room_members(&self, room_id: &str, membership: &str) -> StorageResult<Vec<String>>;
    async fn set_room_alias(&self, alias: &str, room_id: &str, creator: &str) -> StorageResult<()>;
    async fn get_room_alias(&self, alias: &str) -> StorageResult<String>;
    async fn get_room_alias_creator(&self, alias: &str) -> StorageResult<String>;
    async fn delete_room_alias(&self, alias: &str) -> StorageResult<()>;
    async fn get_room_aliases(&self, room_id: &str) -> StorageResult<Vec<String>>;
    async fn set_room_visibility(&self, room_id: &str, visibility: &str) -> StorageResult<()>;
    async fn get_public_rooms(&self, limit: usize, since: Option<&str>, filter: Option<&str>) -> StorageResult<(Vec<PublicRoom>, usize)>;
    async fn forget_room(&self, user_id: &str, room_id: &str) -> StorageResult<()>;

    /// Store a room upgrade edge: old_room --upgrades_to--> new_room.
    async fn store_room_upgrade(&self, old_room_id: &str, new_room_id: &str, version: &str, creator: &str, tombstone_event_id: &str) -> StorageResult<()>;

    /// Get all predecessor room IDs by traversing the upgrade chain backward.
    /// Returns rooms in order from most recent predecessor to oldest.
    async fn get_room_predecessors(&self, room_id: &str) -> StorageResult<Vec<String>>;
}

/// Event storage operations.
#[async_trait]
pub trait EventStore: Send + Sync {
    /// Store an event and return its stream position.
    async fn store_event(&self, event: &StoredEvent) -> StorageResult<i64>;

    /// Get an event by event_id.
    async fn get_event(&self, event_id: &str) -> StorageResult<StoredEvent>;

    /// Get events in a room, ordered by stream_position, with pagination.
    /// `from` is exclusive (events after this position), `dir` is "f" (forward) or "b" (backward).
    async fn get_room_events(&self, room_id: &str, from: i64, limit: usize, dir: &str) -> StorageResult<Vec<StoredEvent>>;

    /// Get all events across all rooms since a stream position (for incremental sync).
    async fn get_events_since(&self, since: i64) -> StorageResult<Vec<StoredEvent>>;

    /// Update the current room state map for a state event.
    async fn set_room_state(&self, room_id: &str, event_type: &str, state_key: &str, event_id: &str) -> StorageResult<()>;

    /// Get the current state events for a room.
    async fn get_current_state(&self, room_id: &str) -> StorageResult<Vec<StoredEvent>>;

    /// Get a specific state event from the current room state.
    async fn get_state_event(&self, room_id: &str, event_type: &str, state_key: &str) -> StorageResult<StoredEvent>;

    /// Get a state event as it was at a given stream position (for departed rooms).
    async fn get_state_event_at(&self, room_id: &str, event_type: &str, state_key: &str, at_position: i64) -> StorageResult<StoredEvent>;

    /// Get the next stream position (atomically incremented).
    async fn next_stream_position(&self) -> StorageResult<i64>;

    /// Get the current (latest) stream position without incrementing.
    async fn current_stream_position(&self) -> StorageResult<i64>;

    /// Store a txn_id -> event_id mapping for deduplication.
    async fn store_txn_id(&self, device_id: &str, txn_id: &str, event_id: &str) -> StorageResult<()>;

    /// Look up an event_id by txn_id for deduplication.
    async fn get_txn_event(&self, device_id: &str, txn_id: &str) -> StorageResult<Option<String>>;

    /// Full-text search across message events in the given rooms.
    /// Returns events matching the query, ordered by relevance.
    async fn search_events(&self, room_ids: &[String], query: &str, limit: usize) -> StorageResult<Vec<StoredEvent>>;

    /// Redact an event — clear its content to `{}`.
    async fn redact_event(&self, event_id: &str) -> StorageResult<()>;
}

/// Read receipt storage.
#[async_trait]
pub trait ReceiptStore: Send + Sync {
    async fn set_receipt(&self, user_id: &str, room_id: &str, receipt_type: &str, event_id: &str) -> StorageResult<()>;
    async fn get_receipts(&self, room_id: &str) -> StorageResult<Vec<ReceiptRecord>>;
}

/// Receipt record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptRecord {
    pub user_id: String,
    pub receipt_type: String,
    pub event_id: String,
    pub ts: u64,
}

/// E2EE key storage.
#[async_trait]
pub trait KeyStore: Send + Sync {
    /// Store/update device keys for a user's device.
    async fn set_device_keys(&self, user_id: &str, device_id: &str, keys: &serde_json::Value) -> StorageResult<()>;

    /// Get device keys for a list of users. Returns map: user_id -> { device_id -> keys }
    async fn get_device_keys(&self, user_ids: &[String]) -> StorageResult<serde_json::Value>;

    /// Store one-time keys. Keys is a map of key_id -> key_data.
    async fn store_one_time_keys(&self, user_id: &str, device_id: &str, keys: &serde_json::Value) -> StorageResult<()>;

    /// Count one-time keys by algorithm for a user's device.
    async fn count_one_time_keys(&self, user_id: &str, device_id: &str) -> StorageResult<serde_json::Value>;

    /// Claim one-time keys. Takes map: user_id -> { device_id -> algorithm }.
    /// Returns map: user_id -> { device_id -> { key_id -> key_data } }.
    /// Claimed keys are deleted from storage.
    async fn claim_one_time_keys(&self, claims: &serde_json::Value) -> StorageResult<serde_json::Value>;

    /// Store cross-signing keys (master, self_signing, user_signing).
    async fn set_cross_signing_keys(&self, user_id: &str, keys: &serde_json::Value) -> StorageResult<()>;

    /// Get cross-signing keys for a user.
    async fn get_cross_signing_keys(&self, user_id: &str) -> StorageResult<serde_json::Value>;
}

/// To-device message storage.
#[async_trait]
pub trait ToDeviceStore: Send + Sync {
    /// Store a to-device message for delivery.
    async fn store_to_device(&self, target_user_id: &str, target_device_id: &str, sender: &str, event_type: &str, content: &serde_json::Value) -> StorageResult<()>;

    /// Get pending to-device messages for a user's device since a position.
    async fn get_to_device_messages(&self, user_id: &str, device_id: &str, since: i64) -> StorageResult<Vec<serde_json::Value>>;

    /// Delete to-device messages up to a position (after client acknowledges via sync).
    async fn delete_to_device_messages(&self, user_id: &str, device_id: &str, up_to: i64) -> StorageResult<()>;
}

/// Account data storage (global and per-room).
#[async_trait]
pub trait AccountDataStore: Send + Sync {
    async fn set_account_data(&self, user_id: &str, room_id: Option<&str>, data_type: &str, content: &serde_json::Value) -> StorageResult<()>;
    async fn get_account_data(&self, user_id: &str, room_id: Option<&str>, data_type: &str) -> StorageResult<serde_json::Value>;
}

/// Media metadata record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRecord {
    pub media_id: String,
    pub server_name: String,
    pub user_id: String,
    pub content_type: String,
    pub content_length: u64,
    pub filename: Option<String>,
    pub s3_key: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub quarantined: bool,
}

/// Media metadata storage.
#[async_trait]
pub trait MediaStore: Send + Sync {
    /// Store media metadata after upload.
    async fn store_media(&self, media: &MediaRecord) -> StorageResult<()>;

    /// Get media metadata by server_name and media_id.
    async fn get_media(&self, server_name: &str, media_id: &str) -> StorageResult<MediaRecord>;

    /// List media uploaded by a user.
    async fn list_user_media(&self, user_id: &str, limit: usize) -> StorageResult<Vec<MediaRecord>>;

    /// Quarantine or unquarantine media.
    async fn set_media_quarantined(&self, server_name: &str, media_id: &str, quarantined: bool) -> StorageResult<()>;

    /// Delete media metadata.
    async fn delete_media(&self, server_name: &str, media_id: &str) -> StorageResult<()>;

    /// List media older than a given timestamp (for retention cleanup).
    async fn list_media_before(&self, before: chrono::DateTime<chrono::Utc>, limit: usize) -> StorageResult<Vec<MediaRecord>>;
}

/// A server signing key record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerKeyRecord {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub private_key: String,
    pub valid_until: chrono::DateTime<chrono::Utc>,
}

/// A cached remote server's public key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteKeyRecord {
    pub server_name: String,
    pub key_id: String,
    pub public_key: String,
    pub valid_until: chrono::DateTime<chrono::Utc>,
}

/// Federation key storage.
#[async_trait]
pub trait FederationKeyStore: Send + Sync {
    async fn store_server_key(&self, key: &ServerKeyRecord) -> StorageResult<()>;
    async fn get_server_key(&self, key_id: &str) -> StorageResult<ServerKeyRecord>;
    async fn get_active_server_keys(&self) -> StorageResult<Vec<ServerKeyRecord>>;
    async fn store_remote_server_keys(&self, keys: &[RemoteKeyRecord]) -> StorageResult<()>;
    async fn get_remote_server_keys(&self, server_name: &str) -> StorageResult<Vec<RemoteKeyRecord>>;
    async fn store_federation_txn(&self, origin: &str, txn_id: &str) -> StorageResult<()>;
    async fn has_federation_txn(&self, origin: &str, txn_id: &str) -> StorageResult<bool>;
}

/// An event relation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationRecord {
    pub event_id: String,
    pub parent_id: String,
    pub room_id: String,
    pub rel_type: String,
    pub sender: String,
    pub event_type: String,
    /// For reactions, the reaction key (e.g. emoji).
    pub content_key: Option<String>,
}

/// An event report record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportRecord {
    pub event_id: String,
    pub room_id: String,
    pub reporter: String,
    pub reason: Option<String>,
    pub score: i64,
}

/// Event relation storage (threads, reactions, edits, references).
#[async_trait]
pub trait RelationStore: Send + Sync {
    /// Store a relation between events.
    async fn store_relation(&self, relation: &RelationRecord) -> StorageResult<()>;

    /// Get relations for a parent event, filtered by rel_type and optionally event_type.
    async fn get_relations(
        &self,
        parent_id: &str,
        rel_type: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
        from: Option<&str>,
    ) -> StorageResult<Vec<RelationRecord>>;

    /// Get aggregated reaction counts for a parent event.
    /// Returns a map of reaction_key -> count.
    async fn get_reaction_counts(&self, parent_id: &str) -> StorageResult<Vec<(String, u64)>>;

    /// Get the latest edit (`m.replace`) for an event, if any.
    async fn get_latest_edit(&self, event_id: &str) -> StorageResult<Option<String>>;

    /// Get thread roots in a room (events that have `m.thread` children).
    async fn get_thread_roots(&self, room_id: &str, limit: usize, from: Option<i64>) -> StorageResult<Vec<String>>;

    /// Store an event report.
    async fn store_report(&self, report: &ReportRecord) -> StorageResult<()>;
}

/// Health check for storage backends.
#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn is_healthy(&self) -> bool;
}

/// Combined storage trait for the complete storage backend.
pub trait Storage: UserStore + DeviceStore + RoomStore + EventStore + ReceiptStore + KeyStore + ToDeviceStore + AccountDataStore + MediaStore + FederationKeyStore + RelationStore + HealthCheck + Send + Sync + 'static {}

/// Blanket implementation.
impl<T> Storage for T where T: UserStore + DeviceStore + RoomStore + EventStore + ReceiptStore + KeyStore + ToDeviceStore + AccountDataStore + MediaStore + FederationKeyStore + RelationStore + HealthCheck + Send + Sync + 'static {}
