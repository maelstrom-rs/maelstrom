//! Real-time notification system connecting event producers to `/sync` consumers.
//!
//! # The problem
//!
//! Matrix's `/sync` endpoint uses long-polling: the client sends a request,
//! and the server holds it open until there is new data to return (or a
//! timeout expires).  This means we need a way for handlers that *produce*
//! events (sending messages, updating typing status, etc.) to *wake up* any
//! `/sync` connections that care about that room or user.
//!
//! # How it works
//!
//! 1. When a handler stores a new event, it calls `state.notifier().notify(...)`.
//! 2. The `/sync` handler calls `state.notifier().subscribe(room_ids, user_id)`
//!    to get a receiver, then `tokio::select!`s between that receiver and a
//!    timeout.
//! 3. When a notification arrives, `/sync` queries storage for the new data and
//!    returns it to the client.
//!
//! # Trait abstraction
//!
//! The [`Notifier`] trait abstracts over the delivery mechanism.  The current
//! implementation ([`LocalNotifier`]) uses in-process `tokio::broadcast`
//! channels, which works for single-node deployments.  For horizontal scaling,
//! you can swap in an implementation backed by a distributed pub/sub layer
//! (e.g. SurrealDB live queries, Redis, or a gossip protocol) without
//! changing any handler code.

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{broadcast, mpsc};

/// A lightweight wake-up signal carrying just enough context for the `/sync`
/// handler to know *what* changed.
///
/// These are intentionally small -- they don't carry event payloads.  The
/// `/sync` handler reads the actual data from storage after being woken up.
#[derive(Debug, Clone)]
pub enum Notification {
    /// A new event was persisted in this room.  Covers messages, state changes,
    /// redactions -- anything that produces a PDU.
    RoomEvent { room_id: String },
    /// The set of currently-typing users changed in this room.
    Typing { room_id: String },
    /// A read receipt (or private read receipt) was sent in this room.
    Receipt { room_id: String },
    /// A user's presence status changed (online, offline, unavailable).
    Presence { user_id: String },
    /// A user's account data changed (push rules, direct chats, custom data).
    AccountData { user_id: String },
}

/// Receiver end of a notification subscription.
///
/// The `/sync` handler holds onto this and `recv().await`s to wait for
/// new events.  When the receiver is dropped, all background forwarding
/// tasks for that subscription are cleaned up automatically (the `mpsc_tx`
/// send fails and the spawned tasks exit).
pub type NotifyReceiver = mpsc::Receiver<Notification>;

/// Trait abstracting over notification delivery.
///
/// This is the seam that lets you swap between in-process channels
/// ([`LocalNotifier`]) and a distributed backend without touching handler code.
///
/// Two publish methods are provided:
///
/// - [`notify`](Notifier::notify) -- async, used from normal handler code.
/// - [`notify_sync`](Notifier::notify_sync) -- synchronous, used from
///   non-async contexts like chitchat/gossip event callbacks where you
///   don't have an async runtime available.
#[async_trait]
pub trait Notifier: Send + Sync + 'static {
    /// Publish a notification asynchronously.
    ///
    /// All subscribers watching the relevant room or user will be woken up.
    async fn notify(&self, notification: Notification);

    /// Publish a notification synchronously (no `.await`).
    ///
    /// For [`LocalNotifier`] this writes directly to the broadcast channel,
    /// which is a non-blocking operation.  Useful in synchronous callbacks
    /// (e.g. cluster gossip event handlers).
    fn notify_sync(&self, notification: Notification);

    /// Subscribe to notifications for a set of rooms and optionally a user.
    ///
    /// Returns an [`mpsc::Receiver`] that yields matching notifications.
    /// The subscription lives as long as the returned receiver is held --
    /// dropping it cleans up all background forwarding tasks.
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

/// In-process notifier backed by `tokio::broadcast` channels.
///
/// # Architecture
///
/// - **One broadcast channel per room**, stored in a [`DashMap`] and created
///   lazily on first subscribe or notify.  `DashMap` is a concurrent hash map
///   that allows lock-free reads, so hot-path lookups don't contend.
/// - **One shared broadcast channel for presence and account data**, since
///   those are keyed by user rather than room.
///
/// # The mpsc adapter pattern
///
/// `broadcast::Receiver` is not convenient for `/sync` because a single
/// `/sync` connection watches *many* rooms plus the user's presence channel.
/// Calling `select!` over a dynamic number of receivers is awkward.
///
/// Instead, [`subscribe`](Notifier::subscribe) creates a single
/// `mpsc::channel` and spawns a small forwarding task for each
/// broadcast subscription.  Each task receives from one broadcast channel
/// and forwards matching notifications into the shared mpsc sender.
/// The `/sync` handler only needs to `recv()` from one receiver.
///
/// When the mpsc receiver is dropped (the `/sync` response completes),
/// the senders fail and the forwarding tasks exit cleanly.
pub struct LocalNotifier {
    /// Broadcast channels per room, created on first subscribe or notify.
    room_channels: DashMap<String, broadcast::Sender<Notification>>,
    /// Single broadcast channel for presence and account-data notifications.
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
            room_channels: DashMap::new(),
            presence_tx,
        }
    }

    fn get_or_create_room_tx(&self, room_id: &str) -> broadcast::Sender<Notification> {
        self.room_channels
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
            Notification::Presence { .. } | Notification::AccountData { .. } => {
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
                loop {
                    match rx.recv().await {
                        Ok(notification) => {
                            if mpsc_tx.send(notification).await.is_err() {
                                break; // receiver dropped
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        // Subscribe to presence and account_data if requested
        if let Some(uid) = user_id {
            let mut rx = self.presence_tx.subscribe();
            let mpsc_tx = mpsc_tx.clone();
            let uid = uid.to_string();

            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(notification) => {
                            let matches = match &notification {
                                Notification::Presence { user_id } => *user_id == uid,
                                Notification::AccountData { user_id } => *user_id == uid,
                                _ => false,
                            };
                            if matches && mpsc_tx.send(notification).await.is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        mpsc_rx
    }
}
