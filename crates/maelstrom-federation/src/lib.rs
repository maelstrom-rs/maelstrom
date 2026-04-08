pub mod backfill;
pub mod client;
pub mod joins;
pub mod key_server;
pub mod receiver;
pub mod router;
pub mod sender;
pub mod signing;
pub mod state;
pub mod user_keys;

use std::sync::Arc;

use maelstrom_core::ephemeral::EphemeralStore;
use maelstrom_core::identifiers::ServerName;
use maelstrom_core::signatures::keys::KeyPair;
use maelstrom_storage::traits::Storage;

/// Shared state for federation endpoints.
#[derive(Clone)]
pub struct FederationState {
    inner: Arc<FederationStateInner>,
}

struct FederationStateInner {
    storage: Box<dyn Storage>,
    ephemeral: Arc<EphemeralStore>,
    signing_key: KeyPair,
    server_name: ServerName,
    federation_client: client::FederationClient,
}

impl FederationState {
    pub fn new(
        storage: impl Storage,
        ephemeral: Arc<EphemeralStore>,
        signing_key: KeyPair,
        server_name: ServerName,
    ) -> Self {
        let fed_client = client::FederationClient::new(signing_key.clone(), server_name.clone());

        Self {
            inner: Arc::new(FederationStateInner {
                storage: Box::new(storage),
                ephemeral,
                signing_key,
                server_name,
                federation_client: fed_client,
            }),
        }
    }

    pub fn storage(&self) -> &dyn Storage {
        &*self.inner.storage
    }

    pub fn ephemeral(&self) -> &EphemeralStore {
        &self.inner.ephemeral
    }

    pub fn signing_key(&self) -> &KeyPair {
        &self.inner.signing_key
    }

    pub fn server_name(&self) -> &ServerName {
        &self.inner.server_name
    }

    pub fn client(&self) -> &client::FederationClient {
        &self.inner.federation_client
    }
}
