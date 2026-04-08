use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

/// A notification signal — lightweight wake-up with optional context.
#[derive(Debug, Clone)]
pub enum Notification {
    /// A new event was stored in this room (message, state change, etc.)
    RoomEvent { room_id: String },
    /// Typing state changed in this room.
    Typing { room_id: String },
    /// A read receipt was sent in this room.
    Receipt { room_id: String },
    /// Presence changed for a user.
    Presence { user_id: String },
}

/// Receiver for notifications. Used by /sync to wait for wake-ups.
pub type NotifyReceiver = mpsc::Receiver<Notification>;

/// Abstraction over notification delivery.
///
/// Currently backed by `tokio::broadcast` channels (in-process only).
/// For multi-node deployments, swap the implementation to use a shared
/// pub/sub layer (e.g. SurrealDB live queries, Redis, or a gossip protocol)
/// without changing any handler code.
#[async_trait]
pub trait Notifier: Send + Sync + 'static {
    /// Publish a notification. All subscribers for the relevant room/user
    /// will be woken up.
    async fn notify(&self, notification: Notification);

    /// Synchronous variant for use in non-async contexts (e.g. chitchat
    /// event callbacks).  Default delegates to the broadcast channel
    /// directly — no `.await` needed for `LocalNotifier`.
    fn notify_sync(&self, notification: Notification);

    /// Subscribe to notifications for a set of rooms and optionally a user (for presence).
    /// Returns a receiver that yields notifications matching the subscription.
    ///
    /// The subscription lives as long as the receiver is held.
    async fn subscribe(&self, room_ids: &[String], user_id: Option<&str>) -> NotifyReceiver;
}

/// Blanket impl so `Box<dyn Notifier>` is itself a `Notifier`.
#[async_trait]
impl Notifier for Box<dyn Notifier> {
    async fn notify(&self, notification: Notification) {
        (**self).notify(notification).await;
    }

    fn notify_sync(&self, notification: Notification) {
        (**self).notify_sync(notification);
    }

    async fn subscribe(&self, room_ids: &[String], user_id: Option<&str>) -> NotifyReceiver {
        (**self).subscribe(room_ids, user_id).await
    }
}

/// Blanket impl so `Arc<dyn Notifier>` is itself a `Notifier`.
#[async_trait]
impl Notifier for std::sync::Arc<dyn Notifier> {
    async fn notify(&self, notification: Notification) {
        (**self).notify(notification).await;
    }

    fn notify_sync(&self, notification: Notification) {
        (**self).notify_sync(notification);
    }

    async fn subscribe(&self, room_ids: &[String], user_id: Option<&str>) -> NotifyReceiver {
        (**self).subscribe(room_ids, user_id).await
    }
}

/// In-process notifier using `tokio::broadcast` channels.
///
/// Each room gets a lazily-created broadcast channel. Presence gets
/// a single shared channel. Subscribers receive a filtered stream
/// via an mpsc adapter.
pub struct LocalNotifier {
    /// Broadcast channels per room, created on first subscribe or notify.
    room_channels: Mutex<HashMap<String, broadcast::Sender<Notification>>>,
    /// Single broadcast channel for presence notifications.
    presence_tx: broadcast::Sender<Notification>,
}

impl Default for LocalNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalNotifier {
    pub fn new() -> Self {
        let (presence_tx, _) = broadcast::channel(256);
        Self {
            room_channels: Mutex::new(HashMap::new()),
            presence_tx,
        }
    }

    fn get_or_create_room_tx(&self, room_id: &str) -> broadcast::Sender<Notification> {
        let mut channels = self.room_channels.lock().unwrap_or_else(|e| e.into_inner());
        channels
            .entry(room_id.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(256);
                tx
            })
            .clone()
    }
}

#[async_trait]
impl Notifier for LocalNotifier {
    async fn notify(&self, notification: Notification) {
        self.notify_sync(notification);
    }

    fn notify_sync(&self, notification: Notification) {
        match &notification {
            Notification::RoomEvent { room_id }
            | Notification::Typing { room_id }
            | Notification::Receipt { room_id } => {
                let tx = self.get_or_create_room_tx(room_id);
                let _ = tx.send(notification);
            }
            Notification::Presence { .. } => {
                let _ = self.presence_tx.send(notification);
            }
        }
    }

    async fn subscribe(&self, room_ids: &[String], user_id: Option<&str>) -> NotifyReceiver {
        let (mpsc_tx, mpsc_rx) = mpsc::channel(64);

        // Subscribe to each room's broadcast channel
        for room_id in room_ids {
            let tx = self.get_or_create_room_tx(room_id);
            let mut rx = tx.subscribe();
            let mpsc_tx = mpsc_tx.clone();

            tokio::spawn(async move {
                while let Ok(notification) = rx.recv().await {
                    if mpsc_tx.send(notification).await.is_err() {
                        break; // receiver dropped
                    }
                }
            });
        }

        // Subscribe to presence if requested
        if let Some(uid) = user_id {
            let mut rx = self.presence_tx.subscribe();
            let mpsc_tx = mpsc_tx.clone();
            let uid = uid.to_string();

            tokio::spawn(async move {
                while let Ok(notification) = rx.recv().await {
                    if let Notification::Presence { user_id } = &notification
                        && *user_id == uid
                            && mpsc_tx.send(notification).await.is_err() {
                                break;
                            }
                }
            });
        }

        mpsc_rx
    }
}
