use actix_cors::Cors;
use actix_web::{middleware::Logger, App, HttpServer};

mod handlers;
mod routes;

#[derive(Clone)]
pub struct Config {
    /// The port and address to run the server on
    pub server_addr: String,
    /// The hostname of the server, used to construct user's id
    pub hostname: String,
    /// Database URL (will distinquish between postgres, sqlite, sled)
    pub database_url: String,
}

impl Config {
    /// Returns a new SeverConfig by attempting
    /// to load from `env` vars.  Panics if
    /// any are missing.
    pub fn new_from_env() -> Self {
        Self {
            server_addr: std::env::var("SERVER_ADDR").expect("SERVER_ADDR env var missing."),
            hostname: std::env::var("HOSTNAME").expect("HOSTNAME env var missing."),
            database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL env var missing."),
        }
    }
}

pub struct State {
    pub config: Config,
}

/// Starts the server. Takes a `ServerConfig`.
pub async fn start(config: Config) -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");
    env_logger::init();

    let addr = config.server_addr.clone();

    HttpServer::new(move || {
        App::new()
            .data(State {
                config: config.clone(),
            })
            .wrap(Cors::new().send_wildcard().finish())
            .wrap(Logger::default())
            .configure(routes::config)
    })
    .bind(addr)?
    .run()
    .await
}
