use super::handlers;
use crate::db::Store;
use actix_web::web;
use actix_web::web::ServiceConfig;

/// Configures the routes/services for Server
pub fn config<T: Store + 'static>(cfg: &mut ServiceConfig) {
    cfg.route(
        "/.well-known/matrix/client",
        web::get().to(handlers::admin::get_wellknown),
    )
    .route(
        "/_matrix/client/versions",
        web::get().to(handlers::admin::get_versions),
    )
    .service(web::scope("/_matrix/client/r0").route(
        "/register",
        web::post().to(handlers::registration::post_register::<T>),
    ));
}
