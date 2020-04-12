use super::handlers;
use crate::db::Store;
use actix_web::web::ServiceConfig;
use actix_web::web::{delete, get, post, resource, scope};

/// Configures the routes/services for Server
pub fn config<T: Store + 'static>(cfg: &mut ServiceConfig) {
    cfg.route(
        "/.well-known/matrix/client",
        get().to(handlers::admin::get_wellknown),
    )
    .route(
        "/_matrix/client/versions",
        get().to(handlers::admin::get_versions),
    )
    .service(
        scope("/_matrix/client/r0")
            .service(
                resource("/register").route(post().to(handlers::registration::post_register::<T>)),
            )
            .service(
                resource("/register/available")
                    .route(get().to(handlers::registration::get_available::<T>)),
            ),
    );
}
