use actix_web::{Error, HttpResponse};
use serde_json::json;

/// Gets discovery information about the domain. The file may include
/// additional keys, which MUST follow the Java package naming convention,
/// e.g. ``com.example.myapp.property``. This ensures property names are
/// suitably namespaced for each application and reduces the risk of
/// clashes.
///
/// Note that this endpoint is not necessarily handled by the homeserver,
/// but by another webserver, to be used for discovering the homeserver URL.
pub async fn get_wellknown() -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok().json("unimplemented!"))
}

/// Gets the versions of the specification supported by the server.
///
/// Values will take the form rX.Y.Z.
///
/// Only the latest Z value will be reported for each supported X.Y value. i.e. if the server
/// implements r0.0.0, r0.0.1, and r1.2.0, it will report r0.0.1 and r1.2.0.
///
/// The server may additionally advertise experimental features it supports through
/// unstable_features. These features should be namespaced and may optionally include version
/// information within their name if desired. Features listed here are not for optionally
/// toggling parts of the Matrix specification and should only be used to advertise support
/// for a feature which has not yet landed in the spec. For example, a feature currently
/// undergoing the proposal process may appear here and eventually be taken off this list
/// once the feature lands in the spec and the server deems it reasonable to do so.
/// Servers may wish to keep advertising features here after they've been released into
/// the spec to give clients a chance to upgrade appropriately. Additionally, clients should
/// avoid using unstable features in their stable releases.
pub async fn get_versions() -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body("{\"versions\":[\"r0.6.0\"]}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{http, test};

    #[actix_rt::test]
    async fn test_get_wellknown_ok() {
        let _req =
            test::TestRequest::with_header("content-type", "application/json").to_http_request();
        let resp = get_wellknown().await;
        assert_eq!(resp.unwrap().status(), http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_versions_ok() {
        let _req =
            test::TestRequest::with_header("content-type", "application/json").to_http_request();
        let resp = get_versions().await;
        assert_eq!(resp.unwrap().status(), http::StatusCode::OK);
    }
}
