use std::time::Duration;

use actix_cors::Cors;
use actix_ratelimit::{MemoryStore, MemoryStoreActor, RateLimiter};
use actix_web::{middleware::Logger, App, HttpServer};
use jsonwebtoken as jwt;

use crate::db;
use crate::CONFIG;

mod error;
mod handlers;
mod routes;

#[derive(Clone)]
pub struct Config {
    /// The port and address to run the server on
    pub server_addr: String,
    /// The hostname of the server, used to construct user's id
    pub hostname: String,
    /// The base url of the server, used to advertise homeserver information
    pub base_url: String,
    /// Database URL (will distinquish between postgres, sqlite, sled)
    pub database_url: String,
    /// ES256 private key for creating auth and session tokens
    pub auth_key: jwt::EncodingKey,
    /// ES256 public key for verifying auth and session tokens (derived from private key)
    pub auth_key_pub: jwt::DecodingKey<'static>,
    /// Verification rules for auth and session tokens
    pub jwt_validation: jwt::Validation,
    /// Duration in seconds that an auth token is valid for
    pub auth_token_expiration: i64,
    /// Duration in seconds that a session token is valid for
    pub session_expiration: i64,
}

impl Config {
    /// Returns a new SeverConfig by attempting
    /// to load from `env` vars.  Panics if
    /// any are missing.
    pub fn new_from_env() -> Self {
        let (auth_key, auth_key_pub) = {
            use std::io::Read;
            let var = std::env::var("AUTH_KEY_FILE").expect("AUTH_KEY_FILE env var missing.");
            let path = std::path::Path::new(&var);
            let mut key_data = Vec::with_capacity(
                path.metadata()
                    .expect("Error fetcing metadata for AUTH_KEY_FILE.")
                    .len() as usize,
            );
            std::fs::File::open(path)
                .expect("Error opening AUTH_KEY_FILE.")
                .read_to_end(&mut key_data)
                .expect("Error reading AUTH_KEY_FILE.");
            crate::util::crypto::parse_keypair(&key_data)
                .expect("Error decoding AUTH_KEY_FILE contents as a PEM encoded ECDSA private key.")
        };
        let hostname = std::env::var("HOSTNAME").expect("HOSTNAME env var missing.");
        Self {
            server_addr: std::env::var("SERVER_ADDR").expect("SERVER_ADDR env var missing."),
            base_url: std::env::var("BASE_URL").expect("BASE_URL env var missing."),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL env var missing."),
            auth_key,
            auth_key_pub,
            jwt_validation: jwt::Validation {
                algorithms: vec![jwt::Algorithm::ES256],
                aud: None,
                iss: Some(hostname.clone()),
                leeway: 5,
                sub: None,
                validate_exp: true,
                validate_nbf: false,
            },
            auth_token_expiration: std::env::var("AUTH_TOKEN_EXPIRATION")
                .expect("SESSION_EXPIRATION env var missing.")
                .parse()
                .expect("Unable to parse SESSION_EXPIRATION as i64."),
            session_expiration: std::env::var("SESSION_EXPIRATION")
                .expect("SESSION_EXPIRATION env var missing.")
                .parse()
                .expect("Unable to parse SESSION_EXPIRATION as i64."),
            hostname,
        }
    }
}

/// Starts the server. Takes a `ServerConfig`.
pub async fn start() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

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
