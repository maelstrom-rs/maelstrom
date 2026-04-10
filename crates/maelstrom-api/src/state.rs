//! Shared application state passed to every Axum handler.
//!
//! Axum requires that the state type you pass to `Router::with_state()` implements
//! `Clone`.  Rather than cloning heavy resources (database pools, HTTP clients),
//! [`AppState`] wraps everything in a single `Arc<AppStateInner>` so cloning is
//! just an atomic reference-count bump -- effectively free.
//!
//! Handlers receive it via `State(state): State<AppState>` and call accessor
//! methods like `state.storage()` or `state.notifier()`.

use std::sync::Arc;

use maelstrom_core::matrix::ephemeral::EphemeralStore;
use maelstrom_core::matrix::id::ServerName;
use maelstrom_federation::client::FederationClient;
use maelstrom_federation::sender::TransactionSender;
use maelstrom_media::client::MediaClient;
use maelstrom_storage::traits::Storage;

use crate::notify::Notifier;

/// Shared application state, available to all Axum handlers via `State<AppState>`.
///
/// Holds every dependency that handlers need at runtime:
///
/// - **`storage`** -- the database layer (SurrealDB). All persistent reads and
///   writes go through the [`Storage`] trait so the backend is swappable.
/// - **`notifier`** -- the pub/sub system that wakes `/sync` long-polls when
///   events arrive. See [`crate::notify::Notifier`].
/// - **`ephemeral`** -- in-memory store for transient data like typing indicators
///   and presence, which don't need to survive restarts.
/// - **`media`** -- optional client for the media backend (RustFS / S3-compatible).
///   `None` when media uploads are disabled.
/// - **`federation`** -- optional client for server-to-server (S2S) operations.
///   `None` when federation is disabled.
/// - **`server_name`** -- this homeserver's server name (e.g. `example.com`),
///   used to construct Matrix IDs like `@alice:example.com`.
/// - **`public_base_url`** -- the externally-reachable URL for this server,
///   used in `.well-known` responses and media download URLs.
/// - **`max_upload_size`** -- media upload size limit in bytes (default 50 MiB).
///
/// # Clone
///
/// `AppState` is cheap to clone because the inner struct is behind an `Arc`.
/// Axum clones the state for every request, so this is important.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    storage: Box<dyn Storage>,
    notifier: Box<dyn Notifier>,
    ephemeral: Arc<EphemeralStore>,
    media: Option<MediaClient>,
    federation: Option<Arc<FederationClient>>,
    transaction_sender: Option<Arc<TransactionSender>>,
    server_name: ServerName,
    public_base_url: String,
    max_upload_size: u64,
}

impl AppState {
    /// Create a new `AppState` with the minimum required dependencies.
    ///
    /// Media and federation are left disabled (`None`).  Use [`Self::with_media`]
    /// or [`Self::with_federation`] to enable them after construction.
    pub fn new(
        storage: impl Storage,
        notifier: impl Notifier,
        ephemeral: Arc<EphemeralStore>,
        server_name: ServerName,
        public_base_url: String,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                storage: Box::new(storage),
                notifier: Box::new(notifier),
                ephemeral,
                media: None,
                federation: None,
                transaction_sender: None,
                server_name,
                public_base_url,
                max_upload_size: 50 * 1024 * 1024, // 50 MiB default
            }),
        }
    }

    /// Create a new `AppState` with media support enabled.
    ///
    /// Use this constructor when the server should accept media uploads and
    /// serve downloads (the `/_matrix/media/` endpoints).
    pub fn with_media(
        storage: impl Storage,
        notifier: impl Notifier,
        ephemeral: Arc<EphemeralStore>,
        media: MediaClient,
        server_name: ServerName,
        public_base_url: String,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                storage: Box::new(storage),
                notifier: Box::new(notifier),
                ephemeral,
                media: Some(media),
                federation: None,
                transaction_sender: None,
                server_name,
                public_base_url,
                max_upload_size: 50 * 1024 * 1024,
            }),
        }
    }

    /// Attach a federation client for S2S operations.
    pub fn with_federation(mut self, client: Arc<FederationClient>) -> Self {
        // Need to rebuild inner since it's Arc-wrapped
        let inner = Arc::get_mut(&mut self.inner).expect("AppState already shared");
        inner.federation = Some(client);
        self
    }

    /// Attach a transaction sender for queuing outbound federation PDUs and EDUs.
    pub fn with_transaction_sender(mut self, sender: Arc<TransactionSender>) -> Self {
        let inner = Arc::get_mut(&mut self.inner).expect("AppState already shared");
        inner.transaction_sender = Some(sender);
        self
    }

    /// Access the database storage backend.
    ///
    /// Every handler that reads or writes persistent data (events, rooms, users,
    /// device keys, etc.) calls this.
    pub fn storage(&self) -> &dyn Storage {
        &*self.inner.storage
    }

    /// Access the notification system.
    ///
    /// Handlers call `notifier().notify(...)` after storing an event to wake
    /// any `/sync` connections that are long-polling for that room or user.
    pub fn notifier(&self) -> &dyn Notifier {
        &*self.inner.notifier
    }

    /// Access the ephemeral (in-memory) store.
    ///
    /// Used for typing indicators, presence, and other transient data that
    /// doesn't need to survive a server restart.
    pub fn ephemeral(&self) -> &EphemeralStore {
        &self.inner.ephemeral
    }

    /// Access the media client, if configured.
    ///
    /// Returns `None` when the server is running without media support.
    /// Handlers should return an appropriate error to the client in that case.
    pub fn media(&self) -> Option<&MediaClient> {
        self.inner.media.as_ref()
    }

    /// Access the federation (server-to-server) client, if configured.
    ///
    /// Returns `None` when federation is disabled. Used by handlers that need
    /// to make outbound requests to other homeservers (e.g. joining remote
    /// rooms, fetching remote user profiles).
    pub fn federation(&self) -> Option<&FederationClient> {
        self.inner.federation.as_deref()
    }

    /// Access the outbound federation transaction sender, if configured.
    ///
    /// Returns `None` when federation is disabled. Used to queue EDUs
    /// (device list updates, typing, presence) for delivery to remote servers.
    pub fn transaction_sender(&self) -> Option<&TransactionSender> {
        self.inner.transaction_sender.as_deref()
    }

    /// This homeserver's server name (e.g. `example.com`).
    ///
    /// Used to construct fully-qualified Matrix IDs (`@user:example.com`,
    /// `!room:example.com`) and to identify this server in federation.
    pub fn server_name(&self) -> &ServerName {
        &self.inner.server_name
    }

    /// The externally-reachable base URL for this server.
    ///
    /// Appears in `.well-known` responses and is used to construct absolute
    /// URLs for media downloads.
    pub fn public_base_url(&self) -> &str {
        &self.inner.public_base_url
    }

    /// Maximum allowed media upload size in bytes.
    ///
    /// Defaults to 50 MiB.  Reported to clients via the `/config` endpoint
    /// so they know the limit before attempting an upload.
    pub fn max_upload_size(&self) -> u64 {
        self.inner.max_upload_size
    }
}
