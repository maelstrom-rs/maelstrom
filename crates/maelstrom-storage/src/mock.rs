use async_trait::async_trait;
use maelstrom_core::events::pdu::StoredEvent;
use maelstrom_core::identifiers::{DeviceId, UserId};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use crate::traits::*;

/// In-memory mock storage for testing.
///
/// Uses `Mutex<HashMap<...>>` internally — not for production use.
#[derive(Debug, Default)]
#[allow(clippy::type_complexity)]
pub struct MockStorage {
    users: Mutex<HashMap<String, UserRecord>>,
    profiles: Mutex<HashMap<String, ProfileRecord>>,
    devices: Mutex<HashMap<String, DeviceRecord>>,
    healthy: Mutex<bool>,
    rooms: Mutex<HashMap<String, RoomRecord>>,
    membership: Mutex<HashMap<(String, String), String>>,
    events: Mutex<Vec<StoredEvent>>,
    room_state: Mutex<HashMap<(String, String, String), String>>,
    txn_ids: Mutex<HashMap<(String, String), String>>,
    stream_position: AtomicI64,
    /// Receipts: (user_id, room_id, receipt_type) -> (event_id, ts)
    receipts: Mutex<HashMap<(String, String, String), (String, u64)>>,
    /// E2EE device keys: (user_id, device_id) -> key data
    device_keys: Mutex<HashMap<(String, String), serde_json::Value>>,
    /// E2EE one-time keys: (user_id, device_id, key_id) -> key data
    one_time_keys: Mutex<HashMap<(String, String, String), serde_json::Value>>,
    /// E2EE cross-signing keys: (user_id, key_type) -> key data
    cross_signing_keys: Mutex<HashMap<(String, String), serde_json::Value>>,
    /// To-device messages: (target_user, target_device, stream_pos, event)
    to_device_messages: Mutex<Vec<(String, String, i64, serde_json::Value)>>,
    /// Room aliases: alias -> (room_id, creator)
    room_aliases: Mutex<HashMap<String, (String, String)>>,
    /// Forgotten rooms: (user_id, room_id)
    forgotten: Mutex<HashSet<(String, String)>>,
    /// Account data: (user_id, room_id_or_empty, data_type) -> content
    account_data: Mutex<HashMap<(String, String, String), serde_json::Value>>,
    /// Media metadata: (server_name, media_id) -> MediaRecord
    media: Mutex<HashMap<(String, String), MediaRecord>>,
    /// Server signing keys: key_id -> ServerKeyRecord
    server_keys: Mutex<HashMap<String, ServerKeyRecord>>,
    /// Remote server keys: server_name -> Vec<RemoteKeyRecord>
    remote_keys: Mutex<HashMap<String, Vec<RemoteKeyRecord>>>,
    /// Federation transaction dedup: (origin, txn_id)
    federation_txns: Mutex<HashSet<(String, String)>>,
    /// Event relations
    relations: Mutex<Vec<RelationRecord>>,
    /// Event reports
    reports: Mutex<Vec<ReportRecord>>,
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

    async fn set_admin(&self, localpart: &str, is_admin: bool) -> StorageResult<()> {
        let mut users = self.users.lock().unwrap();
        let user = users.get_mut(localpart).ok_or(StorageError::NotFound)?;
        user.is_admin = is_admin;
        Ok(())
    }

    async fn count_users(&self) -> StorageResult<u64> {
        Ok(self.users.lock().unwrap().len() as u64)
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

    async fn remove_all_devices_except(&self, user_id: &UserId, keep_device_id: &DeviceId) -> StorageResult<()> {
        let user_str = user_id.to_string();
        let keep = keep_device_id.to_string();
        self.devices
            .lock()
            .unwrap()
            .retain(|_, d| d.user_id != user_str || d.device_id == keep);
        Ok(())
    }

    async fn update_device_display_name(&self, user_id: &UserId, device_id: &DeviceId, display_name: Option<&str>) -> StorageResult<()> {
        let key = format!("{user_id}:{device_id}");
        let mut devices = self.devices.lock().unwrap();
        if let Some(device) = devices.get_mut(&key) {
            device.display_name = display_name.map(|s| s.to_string());
            Ok(())
        } else {
            Err(StorageError::NotFound)
        }
    }
}

#[async_trait]
impl RoomStore for MockStorage {
    async fn create_room(&self, room: &RoomRecord) -> StorageResult<()> {
        let mut rooms = self.rooms.lock().unwrap();
        if rooms.contains_key(&room.room_id) {
            return Err(StorageError::Duplicate(room.room_id.clone()));
        }
        rooms.insert(room.room_id.clone(), room.clone());
        Ok(())
    }

    async fn get_room(&self, room_id: &str) -> StorageResult<RoomRecord> {
        self.rooms
            .lock()
            .unwrap()
            .get(room_id)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn set_membership(&self, user_id: &str, room_id: &str, membership: &str) -> StorageResult<()> {
        self.membership
            .lock()
            .unwrap()
            .insert((user_id.to_string(), room_id.to_string()), membership.to_string());
        Ok(())
    }

    async fn get_membership(&self, user_id: &str, room_id: &str) -> StorageResult<String> {
        self.membership
            .lock()
            .unwrap()
            .get(&(user_id.to_string(), room_id.to_string()))
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn get_joined_rooms(&self, user_id: &str) -> StorageResult<Vec<String>> {
        let membership = self.membership.lock().unwrap();
        Ok(membership
            .iter()
            .filter(|((uid, _), state)| uid == user_id && *state == "join")
            .map(|((_, room_id), _)| room_id.clone())
            .collect())
    }

    async fn get_invited_rooms(&self, user_id: &str) -> StorageResult<Vec<String>> {
        let membership = self.membership.lock().unwrap();
        Ok(membership
            .iter()
            .filter(|((uid, _), state)| uid == user_id && *state == "invite")
            .map(|((_, room_id), _)| room_id.clone())
            .collect())
    }

    async fn get_left_rooms(&self, user_id: &str) -> StorageResult<Vec<String>> {
        let membership = self.membership.lock().unwrap();
        Ok(membership
            .iter()
            .filter(|((uid, _), state)| uid == user_id && *state == "leave")
            .map(|((_, room_id), _)| room_id.clone())
            .collect())
    }

    async fn get_room_members(&self, room_id: &str, membership: &str) -> StorageResult<Vec<String>> {
        let memberships = self.membership.lock().unwrap();
        Ok(memberships
            .iter()
            .filter(|((_, rid), state)| rid == room_id && *state == membership)
            .map(|((user_id, _), _)| user_id.clone())
            .collect())
    }

    async fn set_room_alias(&self, alias: &str, room_id: &str, creator: &str) -> StorageResult<()> {
        let mut aliases = self.room_aliases.lock().unwrap();
        if aliases.contains_key(alias) {
            return Err(StorageError::Duplicate(alias.to_string()));
        }
        aliases.insert(alias.to_string(), (room_id.to_string(), creator.to_string()));
        Ok(())
    }

    async fn get_room_alias(&self, alias: &str) -> StorageResult<String> {
        self.room_aliases
            .lock()
            .unwrap()
            .get(alias)
            .map(|(room_id, _)| room_id.clone())
            .ok_or(StorageError::NotFound)
    }

    async fn get_room_alias_creator(&self, alias: &str) -> StorageResult<String> {
        self.room_aliases
            .lock()
            .unwrap()
            .get(alias)
            .map(|(_, creator)| creator.clone())
            .ok_or(StorageError::NotFound)
    }

    async fn delete_room_alias(&self, alias: &str) -> StorageResult<()> {
        let mut aliases = self.room_aliases.lock().unwrap();
        aliases.remove(alias).ok_or(StorageError::NotFound)?;
        Ok(())
    }

    async fn get_room_aliases(&self, room_id: &str) -> StorageResult<Vec<String>> {
        let aliases = self.room_aliases.lock().unwrap();
        Ok(aliases
            .iter()
            .filter(|(_, (rid, _))| rid == room_id)
            .map(|(alias, _)| alias.clone())
            .collect())
    }

    async fn set_room_visibility(&self, room_id: &str, visibility: &str) -> StorageResult<()> {
        let rooms = self.rooms.lock().unwrap();
        let _room = rooms.get(room_id).ok_or(StorageError::NotFound)?;
        // RoomRecord doesn't have visibility, so we store it as a side-channel
        // For mock, we'll use room_state to track visibility
        drop(rooms);
        // We don't have a visibility field in RoomRecord, so we track it via room_state
        // by convention: (room_id, "__visibility", "") -> visibility
        self.room_state.lock().unwrap().insert(
            (room_id.to_string(), "__visibility".to_string(), String::new()),
            visibility.to_string(),
        );
        Ok(())
    }

    async fn get_public_rooms(&self, limit: usize, since: Option<&str>, filter: Option<&str>) -> StorageResult<(Vec<PublicRoom>, usize)> {
        let rooms = self.rooms.lock().unwrap();
        let room_state = self.room_state.lock().unwrap();
        let membership = self.membership.lock().unwrap();
        let events = self.events.lock().unwrap();

        // Find rooms with visibility = "public"
        let public_room_ids: Vec<String> = rooms
            .keys()
            .filter(|rid| {
                room_state
                    .get(&(rid.to_string(), "__visibility".to_string(), String::new()))
                    .map(|v| v == "public")
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        let total = public_room_ids.len();
        let start = since.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);

        let mut public_rooms: Vec<PublicRoom> = Vec::new();
        for rid in public_room_ids.iter().skip(start).take(limit) {
            // Get name from room_state
            let name = room_state
                .get(&(rid.clone(), "m.room.name".to_string(), String::new()))
                .and_then(|eid| events.iter().find(|e| e.event_id == *eid))
                .and_then(|e| e.content.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()));

            // Get topic from room_state
            let topic = room_state
                .get(&(rid.clone(), "m.room.topic".to_string(), String::new()))
                .and_then(|eid| events.iter().find(|e| e.event_id == *eid))
                .and_then(|e| e.content.get("topic").and_then(|v| v.as_str()).map(|s| s.to_string()));

            // Count joined members
            let num_joined = membership
                .iter()
                .filter(|((_, r), state)| r == rid && *state == "join")
                .count();

            // Check world_readable
            let world_readable = room_state
                .get(&(rid.clone(), "m.room.history_visibility".to_string(), String::new()))
                .and_then(|eid| events.iter().find(|e| e.event_id == *eid))
                .and_then(|e| e.content.get("history_visibility").and_then(|v| v.as_str()))
                .map(|v| v == "world_readable")
                .unwrap_or(false);

            // Check guest_can_join
            let guest_can_join = room_state
                .get(&(rid.clone(), "m.room.guest_access".to_string(), String::new()))
                .and_then(|eid| events.iter().find(|e| e.event_id == *eid))
                .and_then(|e| e.content.get("guest_access").and_then(|v| v.as_str()))
                .map(|v| v == "can_join")
                .unwrap_or(false);

            // Apply filter
            if let Some(filter_str) = filter {
                let filter_lower = filter_str.to_lowercase();
                let matches = name.as_deref().map(|n| n.to_lowercase().contains(&filter_lower)).unwrap_or(false)
                    || topic.as_deref().map(|t| t.to_lowercase().contains(&filter_lower)).unwrap_or(false);
                if !matches {
                    continue;
                }
            }

            // Get canonical alias
            let canonical_alias = room_state
                .get(&(rid.clone(), "m.room.canonical_alias".to_string(), String::new()))
                .and_then(|eid| events.iter().find(|e| e.event_id == *eid))
                .and_then(|e| e.content.get("alias").and_then(|v| v.as_str()).map(|s| s.to_string()));

            // Get avatar URL
            let avatar_url = room_state
                .get(&(rid.clone(), "m.room.avatar".to_string(), String::new()))
                .and_then(|eid| events.iter().find(|e| e.event_id == *eid))
                .and_then(|e| e.content.get("url").and_then(|v| v.as_str()).map(|s| s.to_string()));

            public_rooms.push(PublicRoom {
                room_id: rid.clone(),
                name,
                topic,
                canonical_alias,
                avatar_url,
                num_joined_members: num_joined,
                world_readable,
                guest_can_join,
            });
        }

        Ok((public_rooms, total))
    }

    async fn forget_room(&self, user_id: &str, room_id: &str) -> StorageResult<()> {
        self.forgotten
            .lock()
            .unwrap()
            .insert((user_id.to_string(), room_id.to_string()));
        Ok(())
    }

    async fn store_room_upgrade(&self, old_room_id: &str, new_room_id: &str, _version: &str, _creator: &str, _tombstone_event_id: &str) -> StorageResult<()> {
        // Store upgrade edge in a simple map: new_room -> old_room (predecessor)
        self.room_state.lock().unwrap().insert(
            (new_room_id.to_string(), "__predecessor".to_string(), String::new()),
            old_room_id.to_string(),
        );
        Ok(())
    }

    async fn get_room_predecessors(&self, room_id: &str) -> StorageResult<Vec<String>> {
        let state = self.room_state.lock().unwrap();
        let mut predecessors = Vec::new();
        let mut current = room_id.to_string();
        for _ in 0..100 {
            if let Some(pred) = state.get(&(current.clone(), "__predecessor".to_string(), String::new())) {
                predecessors.push(pred.clone());
                current = pred.clone();
            } else {
                break;
            }
        }
        Ok(predecessors)
    }
}

#[async_trait]
impl EventStore for MockStorage {
    async fn store_event(&self, event: &StoredEvent) -> StorageResult<i64> {
        let pos = self.stream_position.fetch_add(1, Ordering::SeqCst) + 1;
        let mut stored = event.clone();
        stored.stream_position = pos;
        self.events.lock().unwrap().push(stored);
        Ok(pos)
    }

    async fn get_event(&self, event_id: &str) -> StorageResult<StoredEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.event_id == event_id)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn get_room_events(&self, room_id: &str, from: i64, limit: usize, dir: &str) -> StorageResult<Vec<StoredEvent>> {
        let events = self.events.lock().unwrap();
        match dir {
            "b" => {
                // Backward: events in this room with stream_position < from, in reverse order
                let mut result: Vec<StoredEvent> = events
                    .iter()
                    .filter(|e| e.room_id == room_id && e.stream_position < from)
                    .cloned()
                    .collect();
                result.sort_by(|a, b| b.stream_position.cmp(&a.stream_position));
                result.truncate(limit);
                Ok(result)
            }
            _ => {
                // Forward: events in this room with stream_position > from, in order
                let mut result: Vec<StoredEvent> = events
                    .iter()
                    .filter(|e| e.room_id == room_id && e.stream_position > from)
                    .cloned()
                    .collect();
                result.sort_by(|a, b| a.stream_position.cmp(&b.stream_position));
                result.truncate(limit);
                Ok(result)
            }
        }
    }

    async fn get_events_since(&self, since: i64) -> StorageResult<Vec<StoredEvent>> {
        let events = self.events.lock().unwrap();
        let mut result: Vec<StoredEvent> = events
            .iter()
            .filter(|e| e.stream_position > since)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.stream_position.cmp(&b.stream_position));
        Ok(result)
    }

    async fn set_room_state(&self, room_id: &str, event_type: &str, state_key: &str, event_id: &str) -> StorageResult<()> {
        self.room_state
            .lock()
            .unwrap()
            .insert(
                (room_id.to_string(), event_type.to_string(), state_key.to_string()),
                event_id.to_string(),
            );
        Ok(())
    }

    async fn get_current_state(&self, room_id: &str) -> StorageResult<Vec<StoredEvent>> {
        let room_state = self.room_state.lock().unwrap();
        let events = self.events.lock().unwrap();

        let event_ids: Vec<String> = room_state
            .iter()
            .filter(|((rid, _, _), _)| rid == room_id)
            .map(|(_, eid)| eid.clone())
            .collect();

        let mut result = Vec::new();
        for event_id in &event_ids {
            if let Some(event) = events.iter().find(|e| e.event_id == *event_id) {
                result.push(event.clone());
            }
        }
        Ok(result)
    }

    async fn get_state_event(&self, room_id: &str, event_type: &str, state_key: &str) -> StorageResult<StoredEvent> {
        let room_state = self.room_state.lock().unwrap();
        let event_id = room_state
            .get(&(room_id.to_string(), event_type.to_string(), state_key.to_string()))
            .ok_or(StorageError::NotFound)?
            .clone();
        drop(room_state);

        let events = self.events.lock().unwrap();
        events
            .iter()
            .find(|e| e.event_id == event_id)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn get_state_event_at(&self, room_id: &str, event_type: &str, state_key: &str, at_position: i64) -> StorageResult<StoredEvent> {
        let events = self.events.lock().unwrap();
        events
            .iter()
            .filter(|e| {
                e.room_id == room_id
                    && e.event_type == event_type
                    && e.state_key.as_deref() == Some(state_key)
                    && e.stream_position <= at_position
            })
            .max_by_key(|e| e.stream_position)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn next_stream_position(&self) -> StorageResult<i64> {
        Ok(self.stream_position.fetch_add(1, Ordering::SeqCst) + 1)
    }

    async fn current_stream_position(&self) -> StorageResult<i64> {
        Ok(self.stream_position.load(Ordering::SeqCst))
    }

    async fn store_txn_id(&self, device_id: &str, txn_id: &str, event_id: &str) -> StorageResult<()> {
        self.txn_ids
            .lock()
            .unwrap()
            .insert((device_id.to_string(), txn_id.to_string()), event_id.to_string());
        Ok(())
    }

    async fn get_txn_event(&self, device_id: &str, txn_id: &str) -> StorageResult<Option<String>> {
        Ok(self
            .txn_ids
            .lock()
            .unwrap()
            .get(&(device_id.to_string(), txn_id.to_string()))
            .cloned())
    }

    async fn search_events(&self, room_ids: &[String], query: &str, limit: usize) -> StorageResult<Vec<StoredEvent>> {
        let events = self.events.lock().unwrap();
        let query_lower = query.to_lowercase();
        let results: Vec<StoredEvent> = events
            .iter()
            .filter(|e| {
                room_ids.contains(&e.room_id)
                    && e.content
                        .get("body")
                        .and_then(|v| v.as_str())
                        .map(|body| body.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
            })
            .take(limit).cloned()
            .collect();
        Ok(results)
    }

    async fn redact_event(&self, event_id: &str) -> StorageResult<()> {
        let mut events = self.events.lock().unwrap();
        if let Some(event) = events.iter_mut().find(|e| e.event_id == event_id) {
            event.content = serde_json::json!({});
        }
        Ok(())
    }
}

#[async_trait]
impl ReceiptStore for MockStorage {
    async fn set_receipt(&self, user_id: &str, room_id: &str, receipt_type: &str, event_id: &str) -> StorageResult<()> {
        let mut map = self.receipts.lock().unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        map.insert(
            (user_id.to_string(), room_id.to_string(), receipt_type.to_string()),
            (event_id.to_string(), now_ms),
        );
        Ok(())
    }

    async fn get_receipts(&self, room_id: &str) -> StorageResult<Vec<ReceiptRecord>> {
        let map = self.receipts.lock().unwrap();
        Ok(map
            .iter()
            .filter(|((_, rid, _), _)| rid == room_id)
            .map(|((uid, _, rtype), (eid, ts))| ReceiptRecord {
                user_id: uid.clone(),
                receipt_type: rtype.clone(),
                event_id: eid.clone(),
                ts: *ts,
            })
            .collect())
    }
}


#[async_trait]
impl KeyStore for MockStorage {
    async fn set_device_keys(&self, user_id: &str, device_id: &str, keys: &serde_json::Value) -> StorageResult<()> {
        let mut map = self.device_keys.lock().unwrap();
        map.insert((user_id.to_string(), device_id.to_string()), keys.clone());
        Ok(())
    }

    async fn get_device_keys(&self, user_ids: &[String]) -> StorageResult<serde_json::Value> {
        let map = self.device_keys.lock().unwrap();
        let mut result = serde_json::Map::new();
        for uid in user_ids {
            let mut devices = serde_json::Map::new();
            for ((u, d), keys) in map.iter() {
                if u == uid {
                    devices.insert(d.clone(), keys.clone());
                }
            }
            if !devices.is_empty() {
                result.insert(uid.clone(), serde_json::Value::Object(devices));
            }
        }
        Ok(serde_json::Value::Object(result))
    }

    async fn store_one_time_keys(&self, user_id: &str, device_id: &str, keys: &serde_json::Value) -> StorageResult<()> {
        let mut map = self.one_time_keys.lock().unwrap();
        if let Some(obj) = keys.as_object() {
            for (key_id, key_data) in obj {
                map.insert(
                    (user_id.to_string(), device_id.to_string(), key_id.clone()),
                    key_data.clone(),
                );
            }
        }
        Ok(())
    }

    async fn count_one_time_keys(&self, user_id: &str, device_id: &str) -> StorageResult<serde_json::Value> {
        let map = self.one_time_keys.lock().unwrap();
        let mut counts: HashMap<String, i64> = HashMap::new();
        for (u, d, key_id) in map.keys() {
            if u == user_id && d == device_id
                && let Some(algo) = key_id.split(':').next() {
                    *counts.entry(algo.to_string()).or_insert(0) += 1;
                }
        }
        Ok(serde_json::to_value(counts).unwrap_or_default())
    }

    async fn claim_one_time_keys(&self, claims: &serde_json::Value) -> StorageResult<serde_json::Value> {
        let mut map = self.one_time_keys.lock().unwrap();
        let mut result = serde_json::Map::new();

        if let Some(users) = claims.as_object() {
            for (uid, devices) in users {
                let mut user_result = serde_json::Map::new();
                if let Some(devs) = devices.as_object() {
                    for (did, algo_val) in devs {
                        let algo = algo_val.as_str().unwrap_or("");
                        // Find one key matching this user/device/algorithm
                        let found_key = map
                            .keys()
                            .find(|(u, d, kid)| u == uid && d == did && kid.starts_with(&format!("{algo}:")))
                            .cloned();

                        if let Some(key) = found_key {
                            let key_id = key.2.clone();
                            if let Some(key_data) = map.remove(&key) {
                                let mut device_keys = serde_json::Map::new();
                                device_keys.insert(key_id, key_data);
                                user_result.insert(did.clone(), serde_json::Value::Object(device_keys));
                            }
                        }
                    }
                }
                if !user_result.is_empty() {
                    result.insert(uid.clone(), serde_json::Value::Object(user_result));
                }
            }
        }

        Ok(serde_json::Value::Object(result))
    }

    async fn set_cross_signing_keys(&self, user_id: &str, keys: &serde_json::Value) -> StorageResult<()> {
        let mut map = self.cross_signing_keys.lock().unwrap();
        if let Some(obj) = keys.as_object() {
            for (key_type, key_data) in obj {
                map.insert((user_id.to_string(), key_type.clone()), key_data.clone());
            }
        }
        Ok(())
    }

    async fn get_cross_signing_keys(&self, user_id: &str) -> StorageResult<serde_json::Value> {
        let map = self.cross_signing_keys.lock().unwrap();
        let mut result = serde_json::Map::new();
        for ((uid, key_type), key_data) in map.iter() {
            if uid == user_id {
                result.insert(key_type.clone(), key_data.clone());
            }
        }
        Ok(serde_json::Value::Object(result))
    }
}

#[async_trait]
impl ToDeviceStore for MockStorage {
    async fn store_to_device(
        &self,
        target_user_id: &str,
        target_device_id: &str,
        sender: &str,
        event_type: &str,
        content: &serde_json::Value,
    ) -> StorageResult<()> {
        let pos = self.stream_position.fetch_add(1, Ordering::SeqCst) + 1;
        let event = serde_json::json!({
            "sender": sender,
            "type": event_type,
            "content": content,
        });
        self.to_device_messages.lock().unwrap().push((
            target_user_id.to_string(),
            target_device_id.to_string(),
            pos,
            event,
        ));
        Ok(())
    }

    async fn get_to_device_messages(&self, user_id: &str, device_id: &str, since: i64) -> StorageResult<Vec<serde_json::Value>> {
        let msgs = self.to_device_messages.lock().unwrap();
        Ok(msgs
            .iter()
            .filter(|(u, d, pos, _)| u == user_id && d == device_id && *pos > since)
            .map(|(_, _, _, event)| event.clone())
            .collect())
    }

    async fn delete_to_device_messages(&self, user_id: &str, device_id: &str, up_to: i64) -> StorageResult<()> {
        let mut msgs = self.to_device_messages.lock().unwrap();
        msgs.retain(|(u, d, pos, _)| !(u == user_id && d == device_id && *pos <= up_to));
        Ok(())
    }
}

#[async_trait]
impl AccountDataStore for MockStorage {
    async fn set_account_data(&self, user_id: &str, room_id: Option<&str>, data_type: &str, content: &serde_json::Value) -> StorageResult<()> {
        let room_key = room_id.unwrap_or("").to_string();
        self.account_data.lock().unwrap().insert(
            (user_id.to_string(), room_key, data_type.to_string()),
            content.clone(),
        );
        Ok(())
    }

    async fn get_account_data(&self, user_id: &str, room_id: Option<&str>, data_type: &str) -> StorageResult<serde_json::Value> {
        let room_key = room_id.unwrap_or("").to_string();
        self.account_data
            .lock()
            .unwrap()
            .get(&(user_id.to_string(), room_key, data_type.to_string()))
            .cloned()
            .ok_or(StorageError::NotFound)
    }
}

#[async_trait]
impl RelationStore for MockStorage {
    async fn store_relation(&self, relation: &RelationRecord) -> StorageResult<()> {
        self.relations.lock().unwrap().push(relation.clone());
        Ok(())
    }

    async fn get_relations(
        &self,
        parent_id: &str,
        rel_type: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
        _from: Option<&str>,
    ) -> StorageResult<Vec<RelationRecord>> {
        let store = self.relations.lock().unwrap();
        let mut results: Vec<RelationRecord> = store
            .iter()
            .filter(|r| {
                r.parent_id == parent_id
                    && rel_type.is_none_or(|rt| r.rel_type == rt)
                    && event_type.is_none_or(|et| r.event_type == et)
            })
            .cloned()
            .collect();
        results.truncate(limit);
        Ok(results)
    }

    async fn get_reaction_counts(&self, parent_id: &str) -> StorageResult<Vec<(String, u64)>> {
        let store = self.relations.lock().unwrap();
        let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for r in store.iter() {
            if r.parent_id == parent_id && r.rel_type == "m.annotation"
                && let Some(key) = &r.content_key {
                    *counts.entry(key.clone()).or_default() += 1;
                }
        }
        Ok(counts.into_iter().collect())
    }

    async fn get_latest_edit(&self, event_id: &str) -> StorageResult<Option<String>> {
        let store = self.relations.lock().unwrap();
        Ok(store
            .iter()
            .rev()
            .find(|r| r.parent_id == event_id && r.rel_type == "m.replace")
            .map(|r| r.event_id.clone()))
    }

    async fn get_thread_roots(&self, room_id: &str, limit: usize, _from: Option<i64>) -> StorageResult<Vec<String>> {
        let store = self.relations.lock().unwrap();
        let mut roots: Vec<String> = store
            .iter()
            .filter(|r| r.room_id == room_id && r.rel_type == "m.thread")
            .map(|r| r.parent_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        roots.truncate(limit);
        Ok(roots)
    }

    async fn store_report(&self, report: &ReportRecord) -> StorageResult<()> {
        self.reports.lock().unwrap().push(report.clone());
        Ok(())
    }
}

#[async_trait]
impl FederationKeyStore for MockStorage {
    async fn store_server_key(&self, key: &ServerKeyRecord) -> StorageResult<()> {
        let mut store = self.server_keys.lock().unwrap();
        store.insert(key.key_id.clone(), key.clone());
        Ok(())
    }

    async fn get_server_key(&self, key_id: &str) -> StorageResult<ServerKeyRecord> {
        let store = self.server_keys.lock().unwrap();
        store.get(key_id).cloned().ok_or(StorageError::NotFound)
    }

    async fn get_active_server_keys(&self) -> StorageResult<Vec<ServerKeyRecord>> {
        let store = self.server_keys.lock().unwrap();
        Ok(store.values().cloned().collect())
    }

    async fn store_remote_server_keys(&self, keys: &[RemoteKeyRecord]) -> StorageResult<()> {
        let mut store = self.remote_keys.lock().unwrap();
        for key in keys {
            store
                .entry(key.server_name.clone())
                .or_default()
                .push(key.clone());
        }
        Ok(())
    }

    async fn get_remote_server_keys(&self, server_name: &str) -> StorageResult<Vec<RemoteKeyRecord>> {
        let store = self.remote_keys.lock().unwrap();
        Ok(store.get(server_name).cloned().unwrap_or_default())
    }

    async fn store_federation_txn(&self, origin: &str, txn_id: &str) -> StorageResult<()> {
        let mut store = self.federation_txns.lock().unwrap();
        let key = (origin.to_string(), txn_id.to_string());
        if store.contains(&key) {
            return Err(StorageError::Duplicate(format!("{origin}:{txn_id}")));
        }
        store.insert(key);
        Ok(())
    }

    async fn has_federation_txn(&self, origin: &str, txn_id: &str) -> StorageResult<bool> {
        let store = self.federation_txns.lock().unwrap();
        Ok(store.contains(&(origin.to_string(), txn_id.to_string())))
    }
}

#[async_trait]
impl MediaStore for MockStorage {
    async fn store_media(&self, media: &MediaRecord) -> StorageResult<()> {
        let mut store = self.media.lock().unwrap();
        let key = (media.server_name.clone(), media.media_id.clone());
        if store.contains_key(&key) {
            return Err(StorageError::Duplicate(media.media_id.clone()));
        }
        store.insert(key, media.clone());
        Ok(())
    }

    async fn get_media(&self, server_name: &str, media_id: &str) -> StorageResult<MediaRecord> {
        let store = self.media.lock().unwrap();
        store
            .get(&(server_name.to_string(), media_id.to_string()))
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn list_user_media(&self, user_id: &str, limit: usize) -> StorageResult<Vec<MediaRecord>> {
        let store = self.media.lock().unwrap();
        let mut records: Vec<MediaRecord> = store
            .values()
            .filter(|m| m.user_id == user_id)
            .cloned()
            .collect();
        records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        records.truncate(limit);
        Ok(records)
    }

    async fn set_media_quarantined(&self, server_name: &str, media_id: &str, quarantined: bool) -> StorageResult<()> {
        let mut store = self.media.lock().unwrap();
        let key = (server_name.to_string(), media_id.to_string());
        let record = store.get_mut(&key).ok_or(StorageError::NotFound)?;
        record.quarantined = quarantined;
        Ok(())
    }

    async fn delete_media(&self, server_name: &str, media_id: &str) -> StorageResult<()> {
        let mut store = self.media.lock().unwrap();
        let key = (server_name.to_string(), media_id.to_string());
        store.remove(&key).ok_or(StorageError::NotFound)?;
        Ok(())
    }

    async fn list_media_before(&self, before: chrono::DateTime<chrono::Utc>, limit: usize) -> StorageResult<Vec<MediaRecord>> {
        let store = self.media.lock().unwrap();
        let mut records: Vec<MediaRecord> = store
            .values()
            .filter(|m| m.created_at < before)
            .cloned()
            .collect();
        records.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        records.truncate(limit);
        Ok(records)
    }
}

#[async_trait]
impl HealthCheck for MockStorage {
    async fn is_healthy(&self) -> bool {
        *self.healthy.lock().unwrap()
    }
}
