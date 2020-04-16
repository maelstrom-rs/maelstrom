use dotenv;

mod db;
mod models;
mod server;

lazy_static::lazy_static! {
    pub static ref CONFIG: server::Config = server::Config::new_from_env();
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    init_config();

    let _server = server::start().await;

    Ok(())
}

/// Initializes the global config from env vars or `.env` file
pub fn init_config() {
    dotenv::dotenv().ok();
    &*CONFIG; // eagerly load config
}

/// Initialized the global config from an `.env` file name
pub fn init_config_from_file(file_name: &str) {
    dotenv::from_filename(file_name).ok();
    &*CONFIG; // eagerly load config
}
