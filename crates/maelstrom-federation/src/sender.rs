//! # Outbound Federation Transaction Sender
//!
//! When a local user sends a message to a room with remote participants, the event
//! needs to be delivered to every remote server that has members in that room. This
//! module handles that delivery with queuing, batching, and retry.
//!
//! ## Per-Destination Queuing
//!
//! Events are queued separately for each destination server using a [`DashMap`] of
//! `VecDeque`s. PDUs and EDUs have separate queues. This ensures that a slow or
//! unreachable server does not block delivery to other servers.
//!
//! ## Batching
//!
//! The sender drains up to **50 PDUs** and **100 EDUs** per transaction. These are
//! combined into a single `PUT /_matrix/federation/v1/send/{txnId}` request. This
//! matches the Matrix spec recommendation for transaction sizes.
//!
//! ## Exponential Backoff
//!
//! When a transaction fails (network error, remote 5xx, etc.), the PDUs are pushed
//! back to the front of the queue and the destination enters exponential backoff:
//!
//! - First failure: wait **1 second**
//! - Each subsequent failure: **double** the wait time
//! - Maximum wait: **1 hour**
//! - On success: backoff is cleared immediately
//!
//! ## Background Loop
//!
//! The [`TransactionSender::run`] method is designed to be spawned as a long-lived
//! tokio task. It polls all queues every 200ms, skipping destinations that are in
//! backoff.

use std::collections::{HashMap, VecDeque};

use dashmap::DashMap;
use maelstrom_core::matrix::event::{Pdu, timestamp_ms};
use tracing::{debug, info, warn};

use crate::client::FederationClient;

/// Outbound federation transaction sender with per-destination queuing and retry.
///
/// Maintains separate PDU and EDU queues for each destination server, batches them
/// into federation transactions, and retries with exponential backoff on failure.
///
/// # Usage
///
/// Create with [`TransactionSender::new`], then spawn the [`run`](TransactionSender::run)
/// method as a background task. Use [`queue_pdu`](TransactionSender::queue_pdu) and
/// [`queue_edu`](TransactionSender::queue_edu) to enqueue events for delivery.
pub struct TransactionSender {
    client: FederationClient,
    server_name: String,
    queues: DashMap<String, VecDeque<serde_json::Value>>,
    edu_queues: DashMap<String, VecDeque<serde_json::Value>>,
}

impl TransactionSender {
    pub fn new(client: FederationClient, server_name: String) -> Self {
        Self {
            client,
            server_name,
            queues: DashMap::new(),
            edu_queues: DashMap::new(),
        }
    }

    /// Queue a PDU for sending to a destination server.
    ///
    /// The event is serialized to federation JSON format and appended to the
    /// destination's queue. Events addressed to this server are silently dropped.
    pub fn queue_pdu(&self, destination: &str, event: &Pdu) {
        if destination == self.server_name {
            return; // Don't send to ourselves
        }

        let pdu = event.to_federation_json();
        self.queues
            .entry(destination.to_string())
            .or_default()
            .push_back(pdu);

        debug!(destination = %destination, event_id = %event.event_id, "Queued PDU for federation");
    }

    /// Queue an EDU (Ephemeral Data Unit) for sending to a destination server.
    ///
    /// EDUs include typing notifications, presence updates, read receipts, and
    /// device list updates. They are batched alongside PDUs in the next transaction.
    pub fn queue_edu(&self, destination: &str, edu: serde_json::Value) {
        if destination == self.server_name {
            return;
        }

        self.edu_queues
            .entry(destination.to_string())
            .or_default()
            .push_back(edu);
    }

    /// Run the sender loop. Call this as a spawned tokio task.
    ///
    /// This is a long-lived loop that polls every 200ms, draining up to 50 PDUs and
    /// 100 EDUs per destination into a single federation transaction. On failure,
    /// events are re-queued and the destination enters exponential backoff
    /// (1s, 2s, 4s, ... up to 1 hour).
    pub async fn run(self: std::sync::Arc<Self>) {
        info!("Federation transaction sender started");

        let mut backoff: HashMap<String, u64> = HashMap::new();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            let destinations: Vec<String> = {
                let mut dests: std::collections::HashSet<String> = self
                    .queues
                    .iter()
                    .filter(|entry| !entry.value().is_empty())
                    .map(|entry| entry.key().clone())
                    .collect();
                for entry in self.edu_queues.iter() {
                    if !entry.value().is_empty() {
                        dests.insert(entry.key().clone());
                    }
                }
                dests.into_iter().collect()
            };

            for dest in destinations {
                // Check backoff
                if let Some(&retry_at) = backoff.get(&dest) {
                    let now = timestamp_ms();
                    if now < retry_at {
                        continue;
                    }
                }

                // Drain up to 50 PDUs
                let pdus: Vec<serde_json::Value> =
                    if let Some(mut queue) = self.queues.get_mut(&dest) {
                        let count = queue.len().min(50);
                        queue.drain(..count).collect()
                    } else {
                        continue;
                    };

                // Drain up to 100 EDUs
                let edus: Vec<serde_json::Value> =
                    if let Some(mut queue) = self.edu_queues.get_mut(&dest) {
                        let count = queue.len().min(100);
                        queue.drain(..count).collect()
                    } else {
                        Vec::new()
                    };

                if pdus.is_empty() && edus.is_empty() {
                    continue;
                }

                let txn_id = format!("{}_{}", timestamp_ms(), rand::random::<u32>());
                let path = format!("/_matrix/federation/v1/send/{txn_id}");

                let transaction = serde_json::json!({
                    "origin": self.server_name,
                    "origin_server_ts": timestamp_ms(),
                    "pdus": pdus,
                    "edus": edus,
                });

                match self.client.put_json(&dest, &path, &transaction).await {
                    Ok(_) => {
                        debug!(destination = %dest, count = pdus.len(), "Sent federation transaction");
                        backoff.remove(&dest);
                    }
                    Err(e) => {
                        warn!(destination = %dest, error = %e, "Federation send failed");

                        // Re-queue PDUs
                        {
                            let mut queue = self.queues.entry(dest.clone()).or_default();
                            for pdu in pdus.into_iter().rev() {
                                queue.push_front(pdu);
                            }
                        }

                        // Exponential backoff: 1s, 2s, 4s, 8s... up to 1 hour
                        let current_wait = backoff.get(&dest).copied().unwrap_or(0);
                        let next_wait = if current_wait == 0 {
                            1000
                        } else {
                            (current_wait * 2).min(3_600_000)
                        };
                        backoff.insert(dest, timestamp_ms() + next_wait);
                    }
                }
            }
        }
    }
}
