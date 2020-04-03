use dotenv::dotenv;

mod server;

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    let _res = server::api::start("0.0.0.0:8080").await;

    Ok(())
}
