use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use maelstrom_core::events::pdu::{timestamp_ms, StoredEvent};
use tracing::{debug, info, warn};

use crate::client::FederationClient;

/// Outbound federation transaction sender.
///
/// Queues PDUs per destination server, batches them into transactions,
/// and sends with exponential backoff retry.
pub struct TransactionSender {
    client: FederationClient,
    server_name: String,
    queues: Mutex<HashMap<String, VecDeque<serde_json::Value>>>,
    edu_queues: Mutex<HashMap<String, VecDeque<serde_json::Value>>>,
}

impl TransactionSender {
    pub fn new(client: FederationClient, server_name: String) -> Self {
        Self {
            client,
            server_name,
            queues: Mutex::new(HashMap::new()),
            edu_queues: Mutex::new(HashMap::new()),
        }
    }

    /// Queue a PDU for sending to a destination server.
    pub fn queue_pdu(&self, destination: &str, event: &StoredEvent) {
        if destination == self.server_name {
            return; // Don't send to ourselves
        }

        let pdu = event.to_federation_event();
        let mut queues = self.queues.lock().unwrap();
        queues
            .entry(destination.to_string())
            .or_default()
            .push_back(pdu);

        debug!(destination = %destination, event_id = %event.event_id, "Queued PDU for federation");
    }

    /// Queue an EDU for sending to a destination server.
    pub fn queue_edu(&self, destination: &str, edu: serde_json::Value) {
        if destination == self.server_name {
            return;
        }

        let mut queues = self.edu_queues.lock().unwrap();
        queues
            .entry(destination.to_string())
            .or_default()
            .push_back(edu);
    }

    /// Run the sender loop. Call this as a spawned task.
    pub async fn run(self: std::sync::Arc<Self>) {
        info!("Federation transaction sender started");

        let mut backoff: HashMap<String, u64> = HashMap::new();

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            let destinations: Vec<String> = {
                let pdu_queues = self.queues.lock().unwrap();
                let edu_queues = self.edu_queues.lock().unwrap();
                let mut dests: std::collections::HashSet<String> = pdu_queues
                    .iter()
                    .filter(|(_, q)| !q.is_empty())
                    .map(|(dest, _)| dest.clone())
                    .collect();
                for (dest, q) in edu_queues.iter() {
                    if !q.is_empty() {
                        dests.insert(dest.clone());
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
                let pdus: Vec<serde_json::Value> = {
                    let mut queues = self.queues.lock().unwrap();
                    if let Some(queue) = queues.get_mut(&dest) {
                        let count = queue.len().min(50);
                        queue.drain(..count).collect()
                    } else {
                        continue;
                    }
                };

                // Drain up to 100 EDUs
                let edus: Vec<serde_json::Value> = {
                    let mut queues = self.edu_queues.lock().unwrap();
                    if let Some(queue) = queues.get_mut(&dest) {
                        let count = queue.len().min(100);
                        queue.drain(..count).collect()
                    } else {
                        Vec::new()
                    }
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
                            let mut queues = self.queues.lock().unwrap();
                            let queue = queues.entry(dest.clone()).or_default();
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
