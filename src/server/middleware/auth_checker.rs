use std::str::FromStr;

use actix_http::httpmessage::HttpMessage;
use actix_web::{
    dev::ServiceRequest, dev::ServiceResponse, http, http::StatusCode, web::Data, Error,
};

use crate::{
    db::{mock::MockStore, PostgresStore, Store},
    models::auth as auth_model,
    server::error::{ErrorCode, MatrixError},
};
use ruma_identifiers::DeviceId;
use std::task::{Context, Poll};

use std::marker::PhantomData;

use actix_service::{Service, Transform};
use futures::future::{ok, FutureExt, LocalBoxFuture, Ready};

pub struct AuthChecker<T> {
    phantom: PhantomData<T>,
}

impl AuthChecker<MockStore> {
    pub fn mock_store() -> Self {
        AuthChecker {
            phantom: PhantomData::<MockStore>,
        }
    }
}

impl AuthChecker<PostgresStore> {
    pub fn postgres() -> Self {
        AuthChecker {
            phantom: PhantomData::<PostgresStore>,
        }
    }
}

impl<S, B, T> Transform<S> for AuthChecker<T>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    T: 'static + Store,
{
    type Request = ServiceRequest;
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthCheckerMiddleware<S, T>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(AuthCheckerMiddleware {
            service,
            phantom: self.phantom,
        })
    }
}

pub struct AuthCheckerMiddleware<S, T> {
    service: S,
    phantom: PhantomData<T>,
}

fn get_token_from_query(query_string: &str) -> Option<&str> {
    if let Ok(query_map) = serde_urlencoded::from_str::<Vec<(&str, &str)>>(query_string) {
        for (key, value) in query_map {
            if key == "access_token" {
                return Some(value);
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

impl<S, B, T> Service for AuthCheckerMiddleware<S, T>
where
    S: Service<Request = ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
    T: 'static + Store,
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

        let mut device_id_option: Option<DeviceId> = None;
        let mut is_valid = if let Some(token) = auth_token_option {
            if token.is_expired() {
                false
            } else {
                device_id_option = Some(token.device_id.clone());
                req.extensions_mut().insert(token);
                true
            }
        } else {
            false
        };

        let storage: Data<T> = req.app_data().unwrap();
        let fut = self.service.call(req);

        async move {
            is_valid &= if let Some(device_id) = device_id_option {
                storage.check_device_id_exists(&device_id).await.unwrap()
            } else {
                false
            };

            if !is_valid {
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
    use crate::db::mock::MockStore;
    use crate::models::auth::Claims;
    use actix_web::{http, test, web, App, HttpResponse};
    use ruma_identifiers::UserId;

    async fn test_handler() -> Result<HttpResponse, Error> {
        Ok(HttpResponse::build(StatusCode::OK).finish())
    }

    #[actix_rt::test]
    async fn test_header_auth_succeeds() {
        let mut app = test::init_service(
            App::new()
                .data(MockStore::new().with_check_device_id_exists_resp(Ok(true)))
                .route("/", web::get().to(test_handler))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();
        let req = test::TestRequest::get()
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .to_request();
        assert!(app.call(req).await.is_ok());
    }

    #[actix_rt::test]
    async fn test_query_string_auth_succeeds() {
        let mut app = test::init_service(
            App::new()
                .data(MockStore::new().with_check_device_id_exists_resp(Ok(true)))
                .route("/", web::get().to(test_handler))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();
        let req = test::TestRequest::get()
            .uri(format!("/?access_token={}", token).as_str())
            .to_request();
        assert!(app.call(req).await.is_ok());
    }

    #[actix_rt::test]
    async fn test_query_string_auth_fails_empty_db() {
        let mut app = test::init_service(
            App::new()
                .data(MockStore::new().with_check_device_id_exists_resp(Ok(false)))
                .route("/", web::get().to(test_handler))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let token = Claims::auth(
            &UserId::new(&"ruma.io:8080").unwrap(),
            &"some_id".to_owned(),
        )
        .as_jwt()
        .unwrap();
        let req = test::TestRequest::get()
            .uri(format!("/?access_token={}", token).as_str())
            .to_request();
        assert!(app.call(req).await.is_err());
    }

    #[actix_rt::test]
    async fn test_no_auth_fails() {
        let mut app = test::init_service(
            App::new()
                .data(MockStore::new().with_check_device_id_exists_resp(Ok(true)))
                .route("/", web::get().to(test_handler))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let req = test::TestRequest::get().uri("/").to_request();
        assert!(app.call(req).await.is_err());
    }

    #[actix_rt::test]
    async fn test_incorrect_token_fails() {
        let mut app = test::init_service(
            App::new()
                .data(MockStore::new().with_check_device_id_exists_resp(Ok(true)))
                .route("/", web::get().to(test_handler))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/?access_token=token")
            .to_request();
        assert!(app.call(req).await.is_err());
    }
}
