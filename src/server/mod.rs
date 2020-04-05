pub mod api;

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
