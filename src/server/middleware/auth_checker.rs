use std::str::FromStr;

use actix_http::httpmessage::HttpMessage;
use actix_web::{dev::ServiceRequest, dev::ServiceResponse, http, http::StatusCode, Error};

use crate::{
    models::auth as auth_model,
    server::error::{ErrorCode, MatrixError},
};

use std::task::{Context, Poll};

use actix_service::{Service, Transform};
use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};

pub struct AuthChecker;

impl AuthChecker {
    pub fn new() -> Self {
        AuthChecker {}
    }
}

impl<S, B> Transform<S> for AuthChecker
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthCheckerMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(AuthCheckerMiddleware { service })
    }
}

pub struct AuthCheckerMiddleware<S> {
    service: S,
}

fn get_token_from_query(query_string: &str) -> Option<&str> {
    let mut query_map = query_string.split('&');
    for element in &mut query_map {
        let mut pair = element.split('=');
        if let Some(key) = pair.next() {
            if key == "access_token" {
                return pair.next();
            }
        }
    }
    None
}

fn get_token_from_header(header: &str) -> Option<&str> {
    header.split("Bearer ").nth(1)
}

fn get_typed_token(string_repr: &str) -> Option<auth_model::AuthToken> {
    auth_model::AuthToken::from_str(string_repr).ok()
}

impl<S, B> Service for AuthCheckerMiddleware<S>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: ServiceRequest) -> Self::Future {
        let auth_token_option = req
            .headers()
            .get(http::header::AUTHORIZATION)
            .and_then(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(|string| get_token_from_header(string))
            })
            .or_else(|| get_token_from_query(req.query_string()))
            .and_then(|repr| get_typed_token(repr));

        let authorized = if let Some(token) = auth_token_option {
            req.extensions_mut().insert(token);
            true
        } else {
            false
        };
        let fut = self.service.call(req);

        async move {
            if !authorized {
                return Err(MatrixError::new(
                    StatusCode::UNAUTHORIZED,
                    ErrorCode::UNKNOWN_TOKEN,
                    "Unrecognised access token.",
                )
                .into());
            }
            fut.await
        }
        .boxed_local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::auth::Claims;
    use actix_service::IntoService;
    use actix_web::{http, test, HttpResponse};
    use ruma_identifiers::UserId;

    #[actix_rt::test]
    async fn test_header_auth_succeeds() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::build(StatusCode::OK).finish()))
        };
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();
        let mut srv = AuthChecker::new()
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let req = test::TestRequest::get()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .to_srv_request();
        assert!(srv.call(req).await.is_ok());
    }

    #[actix_rt::test]
    async fn test_query_string_auth_succeeds() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::build(StatusCode::OK).finish()))
        };
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();
        let mut srv = AuthChecker::new()
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let req = test::TestRequest::get()
            .uri(format!("/?access_token={}", token).as_str())
            .to_srv_request();
        assert!(srv.call(req).await.is_ok());
    }

    #[actix_rt::test]
    async fn test_no_auth_fails() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::build(StatusCode::OK).finish()))
        };
        let mut srv = AuthChecker::new()
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let req = test::TestRequest::get()
            .uri("/some_method")
            .to_srv_request();
        assert!(srv.call(req).await.is_err());
    }

    #[actix_rt::test]
    async fn test_incorrect_token_fails() {
        let srv = |req: ServiceRequest| {
            ok(req.into_response(HttpResponse::build(StatusCode::OK).finish()))
        };
        let mut srv = AuthChecker::new()
            .new_transform(srv.into_service())
            .await
            .unwrap();
        let req = test::TestRequest::get()
            .uri("/?access_token=token")
            .to_srv_request();
        assert!(srv.call(req).await.is_err());
    }
}
