use std::str::FromStr;

use actix_web::{
    http::StatusCode,
    web::Query,
    Error, HttpResponse,
};
use actix_web_httpauth::extractors::bearer::BearerAuth;

use crate::{
    models::account as account_model,
    models::auth as auth_model,
    server::error::{ErrorCode, MatrixError}
};

fn get_typed_token(string_repr: &str) -> Option<auth_model::AuthToken> {
    auth_model::AuthToken::from_str(string_repr).ok()
}

/// Gets information about the owner of a given access token (i.e. user_id).
///
/// TODO: Rate Limit
///
/// GET /_matrix/client/r0/account/whoami
pub async fn whoami(
    auth: Option<BearerAuth>,
    query: Option<Query<account_model::WhoamiRequest>>,
) -> Result<HttpResponse, Error> {
    auth.and_then(|auth| get_typed_token(auth.token()))
        .or_else(|| query.and_then(|query| get_typed_token(&query.into_inner().access_token)))
        .ok_or_else(|| {
            MatrixError::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::UNKNOWN_TOKEN,
                "Unrecognised access token.",
            )
            .into()
        })
        .map(|token| HttpResponse::Ok().json(account_model::WhoamiResponse { user_id: token.sub }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::auth::Claims;
    use ruma_identifiers::UserId;
    use actix_web::{http, test, web, App};

    #[actix_rt::test]
    async fn test_whoami_with_header_auth_succeeds() {
        crate::init_config_from_file("Settings-test.yml");

        let mut app = test::init_service(
            App::new()
                .route("/whoami", web::get().to(whoami)),
        )
        .await;
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();

        let req = test::TestRequest::get()
            .uri("/whoami")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .to_request();
        let resp = test::call_service(&mut app, req).await;

        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_whoami_with_query_string_auth_suceeds() {
        crate::init_config_from_file("Settings-test.yml");

        let mut app = test::init_service(
            App::new()
                .route("/whoami", web::get().to(whoami)),
        )
        .await;
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();

        let req = test::TestRequest::get()
            .uri(format!("/whoami?access_token={}", token).as_str())
            .header(http::header::CONTENT_TYPE, "application/json")
            .to_request();
        let resp = test::call_service(&mut app, req).await;

        assert!(resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_whoami_without_auth_fails() {
        crate::init_config_from_file("Settings-test.yml");

        let mut app = test::init_service(
            App::new()
                .route("/whoami", web::get().to(whoami)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("whoami")
            .header(http::header::CONTENT_TYPE, "application/json")
            .to_request();
        let resp = test::call_service(&mut app, req).await;
        
        assert!(!resp.status().is_success());
    }

    #[actix_rt::test]
    async fn test_whoami_with_incorrect_token_fails() {
        crate::init_config_from_file("Settings-test.yml");

        let mut app = test::init_service(
            App::new()
                .route("/whoami", web::get().to(whoami)),
        )
        .await;

        let req = test::TestRequest::get()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", "good_enough_token"))
            .header(http::header::CONTENT_TYPE, "application/json")
            .to_request();
        let resp = test::call_service(&mut app, req).await;
        
        assert!(!resp.status().is_success());
    }
}
