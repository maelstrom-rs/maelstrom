use actix_cors::Cors;
use actix_web::{middleware::Logger, App, HttpServer};
use jsonwebtoken as jwt;

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
    /// PEM encoded ES256 key for creating auth tokens
    pub auth_key: jwt::EncodingKey,
    /// Duration in seconds that an auth token is valid for
    pub session_expiration: i64,
}

impl Config {
    /// Returns a new SeverConfig by attempting
    /// to load from `env` vars.  Panics if
    /// any are missing.
    pub fn new_from_env() -> Self {
        Self {
            server_addr: std::env::var("SERVER_ADDR").expect("SERVER_ADDR env var missing."),
            hostname: std::env::var("HOSTNAME").expect("HOSTNAME env var missing."),
            base_url: std::env::var("BASE_URL").expect("BASE_URL env var missing."),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL env var missing."),
            auth_key: {
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
                jwt::EncodingKey::from_ec_pem(&key_data)
                    .expect("Error decoding AUTH_KEY_FILE contents as a PEM encoded ECDSA key.")
            },
            session_expiration: std::env::var("SESSION_EXPIRATION")
                .expect("SESSION_EXPIRATION env var missing.")
                .parse()
                .expect("Unable to parse SESSION_EXPIRATION as i64."),
        }
    }
}

pub struct State {}

/// Starts the server. Takes a `ServerConfig`.
pub async fn start() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let addr = CONFIG.server_addr.clone();

    HttpServer::new(move || {
        App::new()
            .data(State {})
            .wrap(Cors::new().send_wildcard().finish())
            .wrap(Logger::default())
            .configure(routes::config)
    })
    .bind(addr)?
    .run()
    .await
}
