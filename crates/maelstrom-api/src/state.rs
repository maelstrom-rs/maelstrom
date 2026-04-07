use std::sync::Arc;

use maelstrom_core::identifiers::ServerName;
use maelstrom_storage::traits::Storage;

/// Shared application state, available to all Axum handlers via `State<AppState>`.
///
/// This is cloneable (wraps everything in Arc) so Axum can share it across threads.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    storage: Box<dyn Storage>,
    server_name: ServerName,
    public_base_url: String,
}

impl AppState {
    pub fn new(
        storage: impl Storage,
        server_name: ServerName,
        public_base_url: String,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                storage: Box::new(storage),
                server_name,
                public_base_url,
            }),
        }
    }

    pub fn storage(&self) -> &dyn Storage {
        &*self.inner.storage
    }

    pub fn server_name(&self) -> &ServerName {
        &self.inner.server_name
    }

    pub fn public_base_url(&self) -> &str {
        &self.inner.public_base_url
    }
}
