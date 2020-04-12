use dotenv::dotenv;

mod db;
mod models;
mod server;

lazy_static::lazy_static! {
    pub static ref CONFIG: server::Config = server::Config::new_from_env();
}

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    &*CONFIG; // eagerly load config
    let _server = server::start().await;

    Ok(())
}
