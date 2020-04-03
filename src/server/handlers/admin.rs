use actix_web::{HttpResponse, Responder};

/// Gets discovery information about the domain. The file may include
/// additional keys, which MUST follow the Java package naming convention,
/// e.g. ``com.example.myapp.property``. This ensures property names are
/// suitably namespaced for each application and reduces the risk of
/// clashes.
///
/// Note that this endpoint is not necessarily handled by the homeserver,
/// but by another webserver, to be used for discovering the homeserver URL.
pub async fn get_wellknown() -> impl Responder {
    HttpResponse::Ok().json("unimplemented!")
}
