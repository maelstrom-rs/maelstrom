use actix_web::{Error, HttpRequest, HttpResponse};

use crate::{models::account as account_model, models::auth as auth_model};

/// Gets information about the owner of a given access token (i.e. user_id).
///
/// TODO: Rate Limit, Application Service user handling (see https://github.com/matrix-org/dendrite/blob/master/clientapi/auth/auth.go)
///
/// GET /_matrix/client/r0/account/whoami
pub async fn whoami(req: HttpRequest) -> Result<HttpResponse, Error> {
    let token: auth_model::AuthToken = req.extensions_mut().remove().unwrap();
    Ok(HttpResponse::Ok().json(account_model::WhoamiResponse { user_id: token.sub }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::auth::Claims, server::middleware::auth_checker::AuthChecker};

    use actix_web::{http, test, web, App};
    use ruma_identifiers::UserId;
    use serde_json::json;

    use futures_util::stream::StreamExt;

    #[actix_rt::test]
    async fn test_whoami_with_header_auth_succeeds() {
        crate::init_config_from_file("Settings-test.yml");

        let mut app = test::init_service(
            App::new()
                .route("/whoami", web::get().to(whoami))
                .wrap(AuthChecker::new()),
        )
        .await;
        let user_id = UserId::new(&"ruma.io:8080").unwrap();
        let token = Claims::auth(&user_id, &"some_id".to_owned())
            .as_jwt()
            .unwrap();

        let req = test::TestRequest::get()
            .uri("/whoami")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .to_request();
        let mut resp = test::call_service(&mut app, req).await;
        assert!(resp.status().is_success());

        let (bytes, _) = resp.take_body().into_future().await;
        let value: serde_json::Value =
            serde_json::from_slice(bytes.unwrap().unwrap().as_ref()).unwrap();

        assert_eq!(json!({ "user_id": user_id }), value);
    }
}
