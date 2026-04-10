//! Admin API for Maelstrom server management.
//!
//! This crate provides a dual-purpose admin interface:
//!
//! - **JSON API** -- Synapse-compatible REST endpoints under `/_maelstrom/admin/v1/`
//!   for programmatic server management (user CRUD, room moderation, media purging,
//!   federation diagnostics, and server health).
//!
//! - **SSR Dashboard** -- Server-side-rendered HTML pages under `/_maelstrom/admin/`
//!   built with [Askama](https://docs.rs/askama) templates and served alongside
//!   static CSS assets. This gives operators a browser-based overview without
//!   requiring a separate frontend deployment.
//!
//! ## Authentication
//!
//! Every admin endpoint is guarded by the [`auth::AdminUser`] extractor, which
//! validates the `Authorization: Bearer <token>` header exactly like the
//! Client-Server API's `AuthenticatedUser` but additionally checks that the
//! resolved user account has `is_admin = true`. Requests from non-admin users
//! receive a `403 Forbidden` Matrix error.
//!
//! ## State
//!
//! All handlers share an [`AdminState`] that wraps a boxed `Storage` trait object,
//! the server name, process uptime, and a mutable [`RetentionConfig`] that the
//! media retention endpoint can update at runtime.

pub mod auth;
pub mod handlers;
pub mod router;
pub mod templates;

use std::sync::{Arc, Mutex};

use maelstrom_core::matrix::id::ServerName;
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
