//! Device and access token storage -- [`DeviceStore`](crate::traits::DeviceStore) implementation.
//!
//! Devices are stored in the `device` table with a graph edge to their owning
//! user (`device -> belongs_to -> user`).  Access tokens are indexed for O(1)
//! lookup on every authenticated request (`get_device_by_token`).
//!
//! Bulk operations (`remove_all_devices`, `remove_all_devices_except`) are used
//! during logout-all and password-change flows.

use async_trait::async_trait;
use surrealdb::types::{Datetime, RecordId, RecordIdKey, SurrealValue};
use tracing::debug;

use maelstrom_core::matrix::id::{DeviceId, UserId};

use super::SurrealStorage;
use crate::traits::*;

/// Content for creating a device record.
#[derive(Debug, Clone, SurrealValue)]
struct DeviceInput {
    device_id: String,
    user: RecordId,
    display_name: Option<String>,
    access_token: String,
}

/// Row returned when reading a device record.
#[derive(Debug, Clone, SurrealValue)]
struct DeviceRow {
    id: RecordId,
    device_id: String,
    user: RecordId,
    display_name: Option<String>,
    access_token: String,
    created_at: Datetime,
}

/// Extract the string key from a RecordId (e.g. `user:alice` -> `"alice"`).
fn record_key_to_string(rid: &RecordId) -> String {
    match &rid.key {
        RecordIdKey::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

impl DeviceRow {
    fn into_record(self, server_name: &str) -> DeviceRecord {
        let localpart = record_key_to_string(&self.user);
        DeviceRecord {
            device_id: self.device_id,
            user_id: format!("@{localpart}:{server_name}"),
            display_name: self.display_name,
            access_token: self.access_token,
            created_at: self.created_at.into_inner(),
        }
    }

    fn into_record_localpart(self) -> DeviceRecord {
        let localpart = record_key_to_string(&self.user);
        DeviceRecord {
            device_id: self.device_id,
            user_id: localpart,
            display_name: self.display_name,
            access_token: self.access_token,
            created_at: self.created_at.into_inner(),
        }
    }
}

impl SurrealStorage {
    fn user_rid(user_id: &UserId) -> RecordId {
        RecordId::new("user", user_id.localpart())
    }

    fn server_name_from_user_id(user_id: &UserId) -> String {
        user_id.server_name().to_string()
    }
}

#[async_trait]
impl DeviceStore for SurrealStorage {
    async fn create_device(&self, device: &DeviceRecord) -> StorageResult<()> {
        let user_id = UserId::parse(&device.user_id)
            .map_err(|e| StorageError::Query(format!("Invalid user_id: {e}")))?;
        let user_rid = Self::user_rid(&user_id);

        debug!(device_id = %device.device_id, user = %user_id.localpart(), "Creating device");

        let did = device.device_id.clone();
        let lp = user_rid.clone();

        // Delete existing device with same user+device_id if it exists
        self.db()
            .query("DELETE FROM device WHERE user = $user_rid AND device_id = $did")
            .bind(("user_rid", lp))
            .bind(("did", did))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        // Create new device
        let input = DeviceInput {
            device_id: device.device_id.clone(),
            user: user_rid,
            display_name: device.display_name.clone(),
            access_token: device.access_token.clone(),
        };

        let _: Option<serde_json::Value> = self
            .db()
            .create("device")
            .content(input)
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_device(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
    ) -> StorageResult<DeviceRecord> {
        let user_rid = Self::user_rid(user_id);
        let server_name = Self::server_name_from_user_id(user_id);
        let did = device_id.as_str().to_string();

        let mut response = self
            .db()
            .query("SELECT * FROM device WHERE user = $user_rid AND device_id = $did")
            .bind(("user_rid", user_rid))
            .bind(("did", did))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<DeviceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|row| row.into_record(&server_name))
            .ok_or(StorageError::NotFound)
    }

    async fn get_device_by_token(&self, access_token: &str) -> StorageResult<DeviceRecord> {
        let token = access_token.to_string();

        let mut response = self
            .db()
            .query("SELECT * FROM device WHERE access_token = $tok")
            .bind(("tok", token))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<DeviceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        rows.into_iter()
            .next()
            .map(|row| row.into_record_localpart())
            .ok_or(StorageError::NotFound)
    }

    async fn list_devices(&self, user_id: &UserId) -> StorageResult<Vec<DeviceRecord>> {
        let user_rid = Self::user_rid(user_id);
        let server_name = Self::server_name_from_user_id(user_id);

        let mut response = self
            .db()
            .query("SELECT * FROM device WHERE user = $user_rid")
            .bind(("user_rid", user_rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<DeviceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| row.into_record(&server_name))
            .collect())
    }

    async fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) -> StorageResult<()> {
        let user_rid = Self::user_rid(user_id);
        let did = device_id.as_str().to_string();
        debug!(device_id = %device_id, user = %user_id.localpart(), "Removing device");

        self.db()
            .query("DELETE FROM device WHERE user = $user_rid AND device_id = $did")
            .bind(("user_rid", user_rid))
            .bind(("did", did))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn remove_all_devices(&self, user_id: &UserId) -> StorageResult<()> {
        let user_rid = Self::user_rid(user_id);
        debug!(user = %user_id.localpart(), "Removing all devices");

        self.db()
            .query("DELETE FROM device WHERE user = $user_rid")
            .bind(("user_rid", user_rid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn remove_all_devices_except(
        &self,
        user_id: &UserId,
        keep_device_id: &DeviceId,
    ) -> StorageResult<()> {
        let user_rid = Self::user_rid(user_id);
        let keep = keep_device_id.as_str().to_string();
        debug!(user = %user_id.localpart(), keep_device = %keep, "Removing all devices except one");

        self.db()
            .query("DELETE FROM device WHERE user = $user_rid AND device_id != $keep")
            .bind(("user_rid", user_rid))
            .bind(("keep", keep))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn update_device_display_name(
        &self,
        user_id: &UserId,
        device_id: &DeviceId,
        display_name: Option<&str>,
    ) -> StorageResult<()> {
        let user_rid = Self::user_rid(user_id);
        let did = device_id.as_str().to_string();
        let name = display_name.map(|s| s.to_string());

        self.db()
            .query("UPDATE device SET display_name = $name WHERE user = $user_rid AND device_id = $did")
            .bind(("user_rid", user_rid))
            .bind(("did", did))
            .bind(("name", name))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }
}
