use async_trait::async_trait;
use maelstrom_core::identifiers::{DeviceId, UserId};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::traits::*;

/// In-memory mock storage for testing.
///
/// Uses `Mutex<HashMap<...>>` internally — not for production use.
#[derive(Debug, Default)]
pub struct MockStorage {
    users: Mutex<HashMap<String, UserRecord>>,
    profiles: Mutex<HashMap<String, ProfileRecord>>,
    devices: Mutex<HashMap<String, DeviceRecord>>,
    healthy: Mutex<bool>,
}

impl MockStorage {
    pub fn new() -> Self {
        Self {
            healthy: Mutex::new(true),
            ..Default::default()
        }
    }

    pub fn set_healthy(&self, healthy: bool) {
        *self.healthy.lock().unwrap() = healthy;
    }
}

#[async_trait]
impl UserStore for MockStorage {
    async fn create_user(&self, user: &UserRecord) -> StorageResult<()> {
        let mut users = self.users.lock().unwrap();
        if users.contains_key(&user.localpart) {
            return Err(StorageError::Duplicate(user.localpart.clone()));
        }
        users.insert(user.localpart.clone(), user.clone());
        self.profiles.lock().unwrap().insert(
            user.localpart.clone(),
            ProfileRecord {
                display_name: None,
                avatar_url: None,
            },
        );
        Ok(())
    }

    async fn get_user(&self, localpart: &str) -> StorageResult<UserRecord> {
        self.users
            .lock()
            .unwrap()
            .get(localpart)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn user_exists(&self, localpart: &str) -> StorageResult<bool> {
        Ok(self.users.lock().unwrap().contains_key(localpart))
    }

    async fn set_password_hash(&self, localpart: &str, hash: &str) -> StorageResult<()> {
        let mut users = self.users.lock().unwrap();
        let user = users.get_mut(localpart).ok_or(StorageError::NotFound)?;
        user.password_hash = Some(hash.to_string());
        Ok(())
    }

    async fn set_deactivated(&self, localpart: &str, deactivated: bool) -> StorageResult<()> {
        let mut users = self.users.lock().unwrap();
        let user = users.get_mut(localpart).ok_or(StorageError::NotFound)?;
        user.is_deactivated = deactivated;
        Ok(())
    }

    async fn get_profile(&self, localpart: &str) -> StorageResult<ProfileRecord> {
        self.profiles
            .lock()
            .unwrap()
            .get(localpart)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn set_display_name(&self, localpart: &str, name: Option<&str>) -> StorageResult<()> {
        let mut profiles = self.profiles.lock().unwrap();
        let profile = profiles.get_mut(localpart).ok_or(StorageError::NotFound)?;
        profile.display_name = name.map(|s| s.to_string());
        Ok(())
    }

    async fn set_avatar_url(&self, localpart: &str, url: Option<&str>) -> StorageResult<()> {
        let mut profiles = self.profiles.lock().unwrap();
        let profile = profiles.get_mut(localpart).ok_or(StorageError::NotFound)?;
        profile.avatar_url = url.map(|s| s.to_string());
        Ok(())
    }
}

#[async_trait]
impl DeviceStore for MockStorage {
    async fn create_device(&self, device: &DeviceRecord) -> StorageResult<()> {
        let mut devices = self.devices.lock().unwrap();
        let key = format!("{}:{}", device.user_id, device.device_id);
        devices.insert(key, device.clone());
        Ok(())
    }

    async fn get_device(&self, user_id: &UserId, device_id: &DeviceId) -> StorageResult<DeviceRecord> {
        let key = format!("{user_id}:{device_id}");
        self.devices
            .lock()
            .unwrap()
            .get(&key)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn get_device_by_token(&self, access_token: &str) -> StorageResult<DeviceRecord> {
        self.devices
            .lock()
            .unwrap()
            .values()
            .find(|d| d.access_token == access_token)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn list_devices(&self, user_id: &UserId) -> StorageResult<Vec<DeviceRecord>> {
        let user_str = user_id.to_string();
        Ok(self
            .devices
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.user_id == user_str)
            .cloned()
            .collect())
    }

    async fn remove_device(&self, user_id: &UserId, device_id: &DeviceId) -> StorageResult<()> {
        let key = format!("{user_id}:{device_id}");
        self.devices.lock().unwrap().remove(&key);
        Ok(())
    }

    async fn remove_all_devices(&self, user_id: &UserId) -> StorageResult<()> {
        let user_str = user_id.to_string();
        self.devices
            .lock()
            .unwrap()
            .retain(|_, d| d.user_id != user_str);
        Ok(())
    }
}

#[async_trait]
impl HealthCheck for MockStorage {
    async fn is_healthy(&self) -> bool {
        *self.healthy.lock().unwrap()
    }
}
