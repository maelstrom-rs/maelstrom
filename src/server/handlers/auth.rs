use std::borrow::Cow;

use actix_web::{
    http::StatusCode,
    web::{Data, Json},
    Error, HttpRequest, HttpResponse,
};
use jsonwebtoken as jwt;
use ruma_identifiers::{DeviceId, UserId};
use serde_json::json;

use crate::{
    db::Store,
    models::auth as model,
    server::error::{ErrorCode, ResultExt as _},
    CONFIG,
};

lazy_static::lazy_static! {
    pub static ref LOGIN_INFO: String = serde_json::to_string(&json!({
        "flows": &CONFIG.auth_flows,
    })).unwrap();
}

/// Gets the homeserver's supported login types to authenticate users.
/// Clients should pick one of these and supply it as the type when logging in.
///
/// TODO: Rate Limit
///
/// GET /_matrix/client/r0/login
pub async fn login_info() -> Result<HttpResponse, Error> {
    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(&*LOGIN_INFO))
}

pub async fn login<T: Store>(
    req: Json<model::LoginRequest>,
    storage: Data<T>,
) -> Result<HttpResponse, Error> {
    let req = req.into_inner();
    let user_id = storage
        .fetch_user_id(&req.identifier)
        .await
        .unknown()?
        .ok_or("Authentication challenge failed.") // User not found should look identical to auth fail.
        .with_codes(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN)?;
    let device_id = req
        .device_id
        .unwrap_or_else(ruma_identifiers::device_id::generate);
    if let Some(login_type) = req
        .challenge
        .passes(storage.as_ref(), &user_id, &device_id)
        .await
        .unknown()?
    {
        if !CONFIG.auth_flows.contains(&login_type) {
            Err("Authentication challenge failed.")
                .with_codes(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN)?
        }
    };
    let update_dev_id_fut = storage.set_device(
        &user_id,
        &device_id,
        req.initial_device_display_name.as_ref().map(String::as_str),
    );
    let access_token = model::Claims::auth(&user_id, &device_id)
        .as_jwt()
        .with_codes(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::UNKNOWN)?;
    let res = HttpResponse::Ok().json(model::LoginResponse {
        user_id: &user_id,
        access_token: &access_token,
        device_id: &device_id,
        well_known: model::DiscoveryInfo {
            homeserver: model::HomeserverInfo {
                base_url: Cow::Borrowed(&CONFIG.base_url),
            },
        },
    });
    update_dev_id_fut.await.unknown()?;
    Ok(res)
}

pub async fn logout<T: Store>(storage: Data<T>, req: HttpRequest) -> Result<HttpResponse, Error> {
    let token: model::AuthToken = req.extensions_mut().remove().unwrap();
    let remove_device_fut = storage.remove_device_id(&token.device_id, &token.sub);
    let res = HttpResponse::Ok().json(model::LogoutResponse {});
    remove_device_fut.await.unknown()?;
    Ok(res)
}

pub async fn logout_all<T: Store>(
    storage: Data<T>,
    req: HttpRequest,
) -> Result<HttpResponse, Error> {
    let token: model::AuthToken = req.extensions_mut().remove().unwrap();
    let remove_device_fut = storage.remove_all_device_ids(&token.sub);
    let res = HttpResponse::Ok().json(model::LogoutResponse {});
    remove_device_fut.await.unknown()?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::mock::MockStore, models::auth::Claims, server::middleware::auth_checker::AuthChecker,
    };

    use actix_service::Service;
    use actix_web::{http, test, web, App};
    use ruma_identifiers::UserId;

    use futures_util::stream::StreamExt;

    #[actix_rt::test]
    async fn test_logout_succeeds() {
        let mut app = test::init_service(
            App::new()
                .data(
                    MockStore::new()
                        .with_check_device_id_exists_resp(Ok(true))
                        .with_remove_device_id_resp(Ok(())),
                )
                .route("/logout", web::post().to(logout::<MockStore>))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let user_id = UserId::new(&"ruma.io:8080").unwrap();
        let token = Claims::auth(&user_id, &"some_id".to_owned())
            .as_jwt()
            .unwrap();

        let req = test::TestRequest::post()
            .uri("/logout")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .to_request();
        let mut resp = app.call(req).await.unwrap();
        assert!(resp.status().is_success());

        let (bytes, _) = resp.take_body().into_future().await;
        let value: serde_json::Value =
            serde_json::from_slice(bytes.unwrap().unwrap().as_ref()).unwrap();

        assert_eq!(serde_json::value::Value::Null, value);
    }

    #[actix_rt::test]
    async fn test_logout_all_succeeds() {
        let mut app = test::init_service(
            App::new()
                .data(
                    MockStore::new()
                        .with_check_device_id_exists_resp(Ok(true))
                        .with_remove_all_device_ids_resp(Ok(())),
                )
                .route("/logout_all", web::post().to(logout_all::<MockStore>))
                .wrap(AuthChecker::mock_store()),
        )
        .await;
        let user_id = UserId::new(&"ruma.io:8080").unwrap();
        let token = Claims::auth(&user_id, &"some_id".to_owned())
            .as_jwt()
            .unwrap();

        let req = test::TestRequest::post()
            .uri("/logout_all")
            .header(http::header::CONTENT_TYPE, "application/json")
            .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
            .to_request();
        let mut resp = app.call(req).await.unwrap();
        assert!(resp.status().is_success());

        let (bytes, _) = resp.take_body().into_future().await;
        let value: serde_json::Value =
            serde_json::from_slice(bytes.unwrap().unwrap().as_ref()).unwrap();

        assert_eq!(serde_json::value::Value::Null, value);
    }
}
