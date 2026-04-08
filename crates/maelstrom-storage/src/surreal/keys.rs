use async_trait::async_trait;
use surrealdb::types::SurrealValue;
use tracing::debug;

use super::SurrealStorage;
use crate::traits::*;

/// Row returned when reading device key records.
#[derive(Debug, Clone, SurrealValue)]
struct DeviceKeyRow {
    user_id: String,
    device_id: String,
    algorithms: Vec<String>,
    keys: serde_json::Value,
    signatures: serde_json::Value,
}

/// Row returned when reading one-time key records.
#[derive(Debug, Clone, SurrealValue)]
struct OneTimeKeyRow {
    key_id: String,
    key_data: serde_json::Value,
}

/// Row returned when reading cross-signing key records.
#[derive(Debug, Clone, SurrealValue)]
struct CrossSigningKeyRow {
    key_type: String,
    key_data: serde_json::Value,
}

/// Row returned when reading to-device messages.
#[derive(Debug, Clone, SurrealValue)]
struct ToDeviceRow {
    sender: String,
    event_type: String,
    content: serde_json::Value,
    stream_position: i64,
}

#[async_trait]
impl KeyStore for SurrealStorage {
    async fn set_device_keys(
        &self,
        user_id: &str,
        device_id: &str,
        keys: &serde_json::Value,
    ) -> StorageResult<()> {
        debug!(user_id = %user_id, device_id = %device_id, "Setting device keys");

        let algorithms = keys
            .get("algorithms")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let key_map = keys.get("keys").cloned().unwrap_or(serde_json::json!({}));

        let signatures = keys
            .get("signatures")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        let uid = user_id.to_string();
        let did = device_id.to_string();

        // Atomic upsert via transaction: delete existing then create new.
        self.db()
            .query(
                "BEGIN TRANSACTION; \
                 DELETE device_key WHERE user_id = $uid AND device_id = $did; \
                 CREATE device_key SET user_id = $uid, device_id = $did, algorithms = $algos, keys = $keys, signatures = $sigs; \
                 COMMIT TRANSACTION;",
            )
            .bind(("uid", uid))
            .bind(("did", did))
            .bind(("algos", algorithms))
            .bind(("keys", key_map))
            .bind(("sigs", signatures))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_device_keys(&self, user_ids: &[String]) -> StorageResult<serde_json::Value> {
        let mut result = serde_json::Map::new();

        for uid in user_ids {
            let mut response = self
                .db()
                .query("SELECT user_id, device_id, algorithms, keys, signatures FROM device_key WHERE user_id = $uid")
                .bind(("uid", uid.clone()))
                .await
                .map_err(|e| StorageError::Query(e.to_string()))?;

            let rows: Vec<DeviceKeyRow> = response
                .take(0)
                .map_err(|e| StorageError::Query(e.to_string()))?;

            if !rows.is_empty() {
                let mut devices = serde_json::Map::new();
                for row in rows {
                    let device_keys = serde_json::json!({
                        "user_id": row.user_id,
                        "device_id": row.device_id,
                        "algorithms": row.algorithms,
                        "keys": row.keys,
                        "signatures": row.signatures,
                    });
                    devices.insert(row.device_id, device_keys);
                }
                result.insert(uid.clone(), serde_json::Value::Object(devices));
            }
        }

        Ok(serde_json::Value::Object(result))
    }

    async fn store_one_time_keys(
        &self,
        user_id: &str,
        device_id: &str,
        keys: &serde_json::Value,
    ) -> StorageResult<()> {
        debug!(user_id = %user_id, device_id = %device_id, "Storing one-time keys");

        if let Some(obj) = keys.as_object() {
            for (key_id, key_data) in obj {
                let uid = user_id.to_string();
                let did = device_id.to_string();
                let kid = key_id.clone();
                let kdata = key_data.clone();

                // Upsert: delete then create to handle re-uploads
                self.db()
                    .query(
                        "BEGIN TRANSACTION; \
                         DELETE one_time_key WHERE user_id = $uid AND device_id = $did AND key_id = $kid; \
                         CREATE one_time_key SET user_id = $uid, device_id = $did, key_id = $kid, key_data = $kdata; \
                         COMMIT TRANSACTION;",
                    )
                    .bind(("uid", uid))
                    .bind(("did", did))
                    .bind(("kid", kid))
                    .bind(("kdata", kdata))
                    .await
                    .map_err(|e| StorageError::Query(e.to_string()))?;
            }
        }

        Ok(())
    }

    async fn count_one_time_keys(
        &self,
        user_id: &str,
        device_id: &str,
    ) -> StorageResult<serde_json::Value> {
        let mut response = self
            .db()
            .query("SELECT key_id FROM one_time_key WHERE user_id = $uid AND device_id = $did")
            .bind(("uid", user_id.to_string()))
            .bind(("did", device_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<OneTimeKeyRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let mut counts = std::collections::HashMap::<String, i64>::new();
        for row in rows {
            if let Some(algo) = row.key_id.split(':').next() {
                *counts.entry(algo.to_string()).or_insert(0) += 1;
            }
        }

        Ok(serde_json::to_value(counts).unwrap_or_default())
    }

    async fn claim_one_time_keys(
        &self,
        claims: &serde_json::Value,
    ) -> StorageResult<serde_json::Value> {
        let mut result = serde_json::Map::new();

        if let Some(users) = claims.as_object() {
            for (uid, devices) in users {
                let mut user_result = serde_json::Map::new();
                if let Some(devs) = devices.as_object() {
                    for (did, algo_val) in devs {
                        let algo = algo_val.as_str().unwrap_or("");
                        let prefix = format!("{algo}:");

                        // Select one key matching the algorithm, then delete it.
                        let mut response = self
                            .db()
                            .query(
                                "SELECT key_id, key_data FROM one_time_key \
                                 WHERE user_id = $uid AND device_id = $did AND string::starts_with(key_id, $prefix) \
                                 LIMIT 1",
                            )
                            .bind(("uid", uid.clone()))
                            .bind(("did", did.clone()))
                            .bind(("prefix", prefix))
                            .await
                            .map_err(|e| StorageError::Query(e.to_string()))?;

                        let rows: Vec<OneTimeKeyRow> = response
                            .take(0)
                            .map_err(|e| StorageError::Query(e.to_string()))?;

                        if let Some(row) = rows.into_iter().next() {
                            // Delete the claimed key
                            self.db()
                                .query(
                                    "DELETE one_time_key WHERE user_id = $uid AND device_id = $did AND key_id = $kid",
                                )
                                .bind(("uid", uid.clone()))
                                .bind(("did", did.clone()))
                                .bind(("kid", row.key_id.clone()))
                                .await
                                .map_err(|e| StorageError::Query(e.to_string()))?;

                            let mut device_keys = serde_json::Map::new();
                            device_keys.insert(row.key_id, row.key_data);
                            user_result.insert(did.clone(), serde_json::Value::Object(device_keys));
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

    async fn set_cross_signing_keys(
        &self,
        user_id: &str,
        keys: &serde_json::Value,
    ) -> StorageResult<()> {
        debug!(user_id = %user_id, "Setting cross-signing keys");

        let key_types = ["master_key", "self_signing_key", "user_signing_key"];
        if let Some(obj) = keys.as_object() {
            for key_type in &key_types {
                if let Some(key_data) = obj.get(*key_type) {
                    let uid = user_id.to_string();
                    let kt = key_type.to_string();
                    let kdata = key_data.clone();

                    self.db()
                        .query(
                            "BEGIN TRANSACTION; \
                             DELETE cross_signing_key WHERE user_id = $uid AND key_type = $kt; \
                             CREATE cross_signing_key SET user_id = $uid, key_type = $kt, key_data = $kdata; \
                             COMMIT TRANSACTION;",
                        )
                        .bind(("uid", uid))
                        .bind(("kt", kt))
                        .bind(("kdata", kdata))
                        .await
                        .map_err(|e| StorageError::Query(e.to_string()))?;
                }
            }
        }

        Ok(())
    }

    async fn get_cross_signing_keys(&self, user_id: &str) -> StorageResult<serde_json::Value> {
        let mut response = self
            .db()
            .query("SELECT key_type, key_data FROM cross_signing_key WHERE user_id = $uid")
            .bind(("uid", user_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<CrossSigningKeyRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let mut result = serde_json::Map::new();
        for row in rows {
            result.insert(row.key_type, row.key_data);
        }

        Ok(serde_json::Value::Object(result))
    }
}

#[async_trait]
impl ToDeviceStore for SurrealStorage {
    async fn store_to_device(
        &self,
        target_user_id: &str,
        target_device_id: &str,
        sender: &str,
        event_type: &str,
        content: &serde_json::Value,
    ) -> StorageResult<()> {
        debug!(
            target_user = %target_user_id,
            target_device = %target_device_id,
            sender = %sender,
            event_type = %event_type,
            "Storing to-device message"
        );

        let pos = self.next_stream_position().await?;

        self.db()
            .query(
                "CREATE to_device_message SET \
                 target_user_id = $tuid, \
                 target_device_id = $tdid, \
                 sender = $sender, \
                 event_type = $etype, \
                 content = $content, \
                 stream_position = $pos",
            )
            .bind(("tuid", target_user_id.to_string()))
            .bind(("tdid", target_device_id.to_string()))
            .bind(("sender", sender.to_string()))
            .bind(("etype", event_type.to_string()))
            .bind(("content", content.clone()))
            .bind(("pos", pos))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_to_device_messages(
        &self,
        user_id: &str,
        device_id: &str,
        since: i64,
    ) -> StorageResult<Vec<serde_json::Value>> {
        let mut response = self
            .db()
            .query(
                "SELECT sender, event_type, content, stream_position FROM to_device_message \
                 WHERE target_user_id = $uid AND target_device_id = $did AND stream_position > $since \
                 ORDER BY stream_position ASC",
            )
            .bind(("uid", user_id.to_string()))
            .bind(("did", device_id.to_string()))
            .bind(("since", since))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ToDeviceRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "sender": r.sender,
                    "type": r.event_type,
                    "content": r.content,
                })
            })
            .collect())
    }

    async fn delete_to_device_messages(
        &self,
        user_id: &str,
        device_id: &str,
        up_to: i64,
    ) -> StorageResult<()> {
        self.db()
            .query(
                "DELETE to_device_message \
                 WHERE target_user_id = $uid AND target_device_id = $did AND stream_position <= $upto",
            )
            .bind(("uid", user_id.to_string()))
            .bind(("did", device_id.to_string()))
            .bind(("upto", up_to))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }
}
