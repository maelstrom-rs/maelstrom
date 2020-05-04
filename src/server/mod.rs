use std::time::Duration;

use actix_cors::Cors;
use actix_ratelimit::{MemoryStore, MemoryStoreActor, RateLimiter};
use actix_web::{middleware::Logger, App, HttpServer};

use crate::db;
use crate::models::auth::{InteractiveLoginFlow, LoginFlow, LoginType};
use crate::CONFIG;

pub mod error;
mod handlers;
pub mod middleware;
mod routes;

/// Starts the server. Takes a `ServerConfig`.
pub async fn start() -> std::io::Result<()> {
    let addr = CONFIG.server_addr.clone();

    // TODO: Dynamically set db store
    let pg_store = db::PostgresStore::new(&CONFIG.database_url)
        .await
        .expect("Could not establish database connection.");
    // TODO: Support alternative ratelimiting store
    let rl_store = MemoryStore::new();
    let cfg = routes::config::<db::PostgresStore>;

    HttpServer::new(move || {
        App::new()
            .data(pg_store.clone())
            .wrap(
                RateLimiter::new(MemoryStoreActor::from(rl_store.clone()).start())
                    .with_interval(Duration::from_secs(60)) // TODO: Make this a configurable value
                    .with_max_requests(100), // TODO: Make this a configurable value
            )
            .wrap(Cors::new().send_wildcard().finish())
            .wrap(Logger::default())
            .configure(cfg)
    })
    .bind(addr)?
    .run()
    .await
}
