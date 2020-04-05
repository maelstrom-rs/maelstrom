use dotenv::dotenv;

mod models;
mod server;

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    let config = server::Config::new_from_env();
    let _server = server::start(config).await;

    Ok(())
}
