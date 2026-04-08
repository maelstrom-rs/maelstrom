pub mod auth;
pub mod handlers;
pub mod router;
pub mod templates;

use std::sync::{Arc, Mutex};

use maelstrom_core::identifiers::ServerName;
use maelstrom_storage::traits::Storage;

/// Media retention policy configuration.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub max_age_days: u64,
    pub sweep_interval_secs: u64,
    pub batch_size: usize,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_age_days: 0,
            sweep_interval_secs: 3600,
            batch_size: 500,
        }
    }
}

/// Shared state for admin endpoints.
#[derive(Clone)]
pub struct AdminState {
    inner: Arc<AdminStateInner>,
}

struct AdminStateInner {
    storage: Box<dyn Storage>,
    server_name: ServerName,
    start_time: std::time::Instant,
    retention_config: Mutex<RetentionConfig>,
}

impl AdminState {
    pub fn new(storage: impl Storage, server_name: ServerName) -> Self {
        Self {
            inner: Arc::new(AdminStateInner {
                storage: Box::new(storage),
                server_name,
                start_time: std::time::Instant::now(),
                retention_config: Mutex::new(RetentionConfig::default()),
            }),
        }
    }

    pub fn with_retention(
        storage: impl Storage,
        server_name: ServerName,
        retention: RetentionConfig,
    ) -> Self {
        Self {
            inner: Arc::new(AdminStateInner {
                storage: Box::new(storage),
                server_name,
                start_time: std::time::Instant::now(),
                retention_config: Mutex::new(retention),
            }),
        }
    }

    pub fn storage(&self) -> &dyn Storage {
        &*self.inner.storage
    }

    pub fn server_name(&self) -> &ServerName {
        &self.inner.server_name
    }

    pub fn uptime_secs(&self) -> u64 {
        self.inner.start_time.elapsed().as_secs()
    }

    pub fn retention_config(&self) -> RetentionConfig {
        self.inner.retention_config.lock().unwrap().clone()
    }

    pub fn set_retention_config(&self, config: RetentionConfig) {
        *self.inner.retention_config.lock().unwrap() = config;
    }
}
