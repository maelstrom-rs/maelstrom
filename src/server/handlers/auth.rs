use std::borrow::Cow;

use actix_web::{
    http::StatusCode,
    web::{Data, Json},
    Error, HttpResponse,
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
        "flows": vec![
            model::LoginFlow {
                login_type: model::LoginType::Password
            },
            model::LoginFlow {
                login_type: model::LoginType::Token
            }
        ]
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
    if !req
        .challenge
        .passes(storage.as_ref(), None, &user_id, &device_id)
        .await
        .unknown()?
    {
        Err("Authentication challenge failed.")
            .with_codes(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN)?
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
