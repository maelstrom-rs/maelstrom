use async_trait::async_trait;
use surrealdb::types::SurrealValue;

use super::SurrealStorage;
use crate::traits::*;

/// Row returned when reading receipt records.
#[derive(Debug, Clone, SurrealValue)]
struct ReceiptRow {
    user_id: String,
    receipt_type: String,
    event_id: String,
    ts: i64,
}

#[async_trait]
impl ReceiptStore for SurrealStorage {
    async fn set_receipt(
        &self,
        user_id: &str,
        room_id: &str,
        receipt_type: &str,
        event_id: &str,
    ) -> StorageResult<()> {
        let uid = user_id.to_string();
        let rid = room_id.to_string();
        let rtype = receipt_type.to_string();
        let eid = event_id.to_string();

        // Atomic upsert via transaction: delete existing then create new.
        self.db()
            .query(
                "BEGIN TRANSACTION; \
                 DELETE receipt WHERE user_id = $uid AND room_id = $rid AND receipt_type = $rtype; \
                 CREATE receipt SET user_id = $uid, room_id = $rid, receipt_type = $rtype, event_id = $eid, ts = time::millis(time::now()); \
                 COMMIT TRANSACTION;",
            )
            .bind(("uid", uid))
            .bind(("rid", rid))
            .bind(("rtype", rtype))
            .bind(("eid", eid))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(())
    }

    async fn get_receipts(&self, room_id: &str) -> StorageResult<Vec<ReceiptRecord>> {
        let mut response = self
            .db()
            .query("SELECT user_id, receipt_type, event_id, ts FROM receipt WHERE room_id = $rid")
            .bind(("rid", room_id.to_string()))
            .await
            .map_err(|e| StorageError::Query(e.to_string()))?;

        let rows: Vec<ReceiptRow> = response
            .take(0)
            .map_err(|e| StorageError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| ReceiptRecord {
                user_id: r.user_id,
                receipt_type: r.receipt_type,
                event_id: r.event_id,
                ts: r.ts as u64,
            })
            .collect())
    }
}
