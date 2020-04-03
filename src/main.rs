use dotenv::dotenv;

#[actix_rt::main]
async fn main() -> std::io::Result<()> {
    dotenv().ok();

    println!("Hello, world!");

    Ok(())
}
