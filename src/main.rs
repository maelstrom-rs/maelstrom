use dotenv::dotenv;
use server::api;

mod server;

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    let config = api::ServerConfig::new_from_env();
    let _server = api::start(config).await;

    Ok(())
}
