use std::borrow::Cow;

use actix_web::{http::StatusCode, web::Json, Error, HttpResponse};
use jsonwebtoken as jwt;
use ruma_identifiers::{DeviceId, UserId};
use serde_json::json;

use crate::{
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

pub async fn login(req: Json<model::LoginRequest>) -> Result<HttpResponse, Error> {
    let user_id = match &req.challenge {
        model::Challenge::Password { password } => {
            unimplemented!("check password against user db") // TODO: will finish once user db model is complete
        }
        model::Challenge::Token { token } => {
            unimplemented!("check OTP against user db") // TODO: will finish once user db model is complete
        }
    };
    let device_id = ruma_identifiers::device_id::generate(); // TODO: implement method of finding existing device_id and verifying generated id does not collide
    let access_token = jwt::encode(
        &jwt::Header::new(jwt::Algorithm::ES256),
        &Claims::new(&user_id, &device_id),
        &CONFIG.auth_key,
    )
    .with_codes(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::UNKNOWN)?;
    Ok(HttpResponse::Ok().json(model::LoginResponse {
        user_id,
        access_token,
        device_id,
        well_known: model::DiscoveryInfo {
            homeserver: model::HomeserverInfo {
                base_url: Cow::Borrowed(&CONFIG.base_url),
            },
        },
    }))
}
