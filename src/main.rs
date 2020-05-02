mod config;
mod db;
mod models;
mod server;
mod util;

lazy_static::lazy_static! {
    pub static ref CONFIG: crate::config::Config = {
        let maybe_config = crate::config::Config::load();
        maybe_config.unwrap()
    };
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info,maelstrom=info");
    env_logger::init();
    init_config();

    let _server = server::start().await;

    Ok(())
}

/// Initializes the global config from env vars or `.env` file
pub fn init_config() {
    &*CONFIG; // eagerly load config
}

/// Initializes the global config from an `.env` file name
pub fn init_config_from_file(file_name: &str) {
    std::env::set_var("MAELSTROM_CONFIG_FILE", file_name);
    &*CONFIG; // eagerly load config
}
