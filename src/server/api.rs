use super::routes;
use actix_web::{App, HttpServer};

pub async fn start(addr: &str) -> std::io::Result<()> {
    HttpServer::new(|| App::new().configure(routes::router_config))
        .bind(addr)?
        .run()
        .await
}
