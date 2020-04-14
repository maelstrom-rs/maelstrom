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

#[derive(Clone, Debug, serde::Serialize)]
pub struct Claims<'a, 'b> {
    pub iss: &'static str,
    pub iat: i64,
    pub exp: i64,
    pub sub: &'a UserId,
    pub device_id: &'b DeviceId,
}
impl<'a, 'b> Claims<'a, 'b> {
    pub fn new(user_id: &'a UserId, device_id: &'b DeviceId) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|a| a.as_secs() as i64)
            .unwrap_or_else(|a| -(a.duration().as_secs() as i64));
        Self {
            iss: &CONFIG.hostname,
            iat: now,
            exp: now + CONFIG.session_expiration,
            sub: user_id,
            device_id,
        }
    }
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
        .ok_or("Authentication challenge failed.")
        .with_codes(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN)?;
    match &req.challenge {
        model::Challenge::Password { password } => {
            let pwhash = storage.fetch_password_hash(&user_id).await.unknown()?;
            if !pwhash.matches(&password) {
                Err("Authentication challenge failed.")
                    .with_codes(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN)?
            }
        }
        model::Challenge::Token { token } => {
            if !storage.check_otp_exists(&user_id, token).await.unknown()? {
                Err("Authentication challenge failed.")
                    .with_codes(StatusCode::FORBIDDEN, ErrorCode::FORBIDDEN)?
            }
        }
    };
    let device_id = req
        .device_id
        .unwrap_or_else(ruma_identifiers::device_id::generate);
    let update_dev_id_fut = storage.set_device(
        &user_id,
        &device_id,
        req.initial_device_display_name.as_ref().map(String::as_str),
    );
    let access_token = jwt::encode(
        &jwt::Header::new(jwt::Algorithm::ES256),
        &Claims::new(&user_id, &device_id),
        &CONFIG.auth_key,
    )
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
