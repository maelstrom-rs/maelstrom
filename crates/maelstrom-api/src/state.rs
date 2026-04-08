use std::sync::Arc;

use maelstrom_core::ephemeral::EphemeralStore;
use maelstrom_core::identifiers::ServerName;
use maelstrom_media::client::MediaClient;
use maelstrom_storage::traits::Storage;

use crate::notify::Notifier;

/// Shared application state, available to all Axum handlers via `State<AppState>`.
///
/// This is cloneable (wraps everything in Arc) so Axum can share it across threads.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    storage: Box<dyn Storage>,
    notifier: Box<dyn Notifier>,
    ephemeral: Arc<EphemeralStore>,
    media: Option<MediaClient>,
    server_name: ServerName,
    public_base_url: String,
    max_upload_size: u64,
}

impl AppState {
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
                server_name,
                public_base_url,
                max_upload_size: 50 * 1024 * 1024, // 50 MiB default
            }),
        }
    }

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
                server_name,
                public_base_url,
                max_upload_size: 50 * 1024 * 1024,
            }),
        }
    }

    pub fn storage(&self) -> &dyn Storage {
        &*self.inner.storage
    }

    pub fn notifier(&self) -> &dyn Notifier {
        &*self.inner.notifier
    }

    pub fn ephemeral(&self) -> &EphemeralStore {
        &self.inner.ephemeral
    }

    pub fn media(&self) -> Option<&MediaClient> {
        self.inner.media.as_ref()
    }

    pub fn server_name(&self) -> &ServerName {
        &self.inner.server_name
    }

    pub fn public_base_url(&self) -> &str {
        &self.inner.public_base_url
    }

    pub fn max_upload_size(&self) -> u64 {
        self.inner.max_upload_size
    }
}
