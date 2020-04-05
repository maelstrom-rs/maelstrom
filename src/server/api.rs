use super::{routes, Config, State};
use actix_cors::Cors;
use actix_web::{middleware::Logger, App, HttpServer};

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
