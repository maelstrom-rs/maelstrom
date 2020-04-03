use actix_web::web;
use actix_web::web::ServiceConfig;

/// Configures the routes/services for Server
pub fn router_config(cfg: &mut ServiceConfig) {
    cfg.service(web::scope("/_matrix/client/r0"));
}
