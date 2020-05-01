use std::borrow::{Borrow, Cow};
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

use jsonwebtoken as jwt;
use linear_map::{set::LinearSet, LinearMap};
use ruma_identifiers::{DeviceId, UserId};

use crate::db::{Error as StorageError, Store};
use crate::CONFIG;

#[derive(Clone, Debug)]
pub enum Error {
    Storage(StorageError),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Storage(e) => write!(f, "Storage Error: {}", e),
        }
    }
}
impl From<StorageError> for Error {
    fn from(e: StorageError) -> Self {
        Error::Storage(e)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenKind {
    Session,
    Auth,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Claims<'a> {
    pub kind: TokenKind,
    pub iss: &'static str,
    pub iat: u64,
    pub exp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<u32>,
    pub sub: &'a UserId,
    pub device_id: &'a DeviceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<&'a [LoginType]>,
}
impl<'a> Claims<'a> {
    pub fn auth(user_id: &'a UserId, device_id: &'a DeviceId) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|a| a.as_secs())
            .unwrap_or_else(|a| 0);
        Self {
            kind: TokenKind::Auth,
            iss: &CONFIG.hostname,
            iat: now,
            exp: now.saturating_add(CONFIG.auth_token_expiration.as_secs()),
            jti: Some(rand::random()),
            sub: user_id,
            device_id,
            complete: None,
        }
    }

    pub fn session(
        user_id: &'a UserId,
        device_id: &'a DeviceId,
        complete: &'a [LoginType],
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|a| a.as_secs())
            .unwrap_or_else(|a| 0);
        Self {
            kind: TokenKind::Session,
            iss: &CONFIG.hostname,
            iat: now,
            exp: now.saturating_add(CONFIG.auth_token_expiration.as_secs()),
            jti: None,
            sub: user_id,
            device_id,
            complete: Some(complete),
        }
    }

    pub fn as_jwt(&self) -> Result<String, jwt::errors::Error> {
        jwt::encode(
            &CONFIG.jwt_config.jwt_header,
            &self,
            &CONFIG.jwt_config.auth_key,
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SessionToken {
    pub sub: UserId,
    pub device_id: DeviceId,
    pub complete: Vec<LoginType>,
}
impl FromStr for SessionToken {
    type Err = jwt::errors::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let data = jwt::decode(
            s,
            &CONFIG.jwt_config.auth_key_pub,
            &CONFIG.jwt_config.jwt_validation,
        )?;
        Ok(data.claims)
    }
}
impl SessionToken {
    pub async fn update<T: Store>(
        &mut self,
        store: &T,
        challenge: &Challenge,
    ) -> Result<bool, Error> {
        if let Some(login_type) = challenge.passes(store, &self.sub, &self.device_id).await? {
            self.complete.push(login_type);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn is_complete(&self) -> bool {
        CONFIG
            .interactive_auth_flows
            .contains::<[LoginType]>(&self.complete)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct AuthToken {
    pub sub: UserId,
    pub device_id: DeviceId,
    pub jti: u32,
}
impl FromStr for AuthToken {
    type Err = jwt::errors::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let data = jwt::decode(
            s,
            &CONFIG.jwt_config.auth_key_pub,
            &CONFIG.jwt_config.jwt_validation,
        )?;
        Ok(data.claims)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct InteractiveLoginFlow {
    pub stages: Vec<LoginType>,
}
impl Borrow<[LoginType]> for InteractiveLoginFlow {
    fn borrow(&self) -> &[LoginType] {
        self.stages.as_slice()
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct InteractiveAuth {
    #[serde(flatten)]
    pub challenge: Challenge,
    #[serde(deserialize_with = "crate::util::serde::deser_parse")]
    pub session: SessionToken,
}
impl InteractiveAuth {
    pub async fn handle<T: Store>(mut self, store: &T) -> Result<(), actix_web::Error> {
        use crate::server::error::{ErrorCode, ResultExt};
        use actix_web::{http::StatusCode, HttpResponse};

        #[derive(serde::Serialize)]
        struct IncompleteAuthResponse<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            errcode: Option<ErrorCode>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error: Option<&'static str>,
            completed: &'a [LoginType],
            flows: &'a LinearSet<InteractiveLoginFlow>,
            params: &'a LinearMap<LoginType, serde_json::Value>,
            session: &'a str,
        }

        let success = self
            .session
            .update(store, &self.challenge)
            .await
            .unknown()?;
        if self.session.is_complete() {
            Ok(())
        } else {
            Err(HttpResponse::build(StatusCode::UNAUTHORIZED)
                .json(&IncompleteAuthResponse {
                    errcode: if success {
                        None
                    } else {
                        Some(ErrorCode::FORBIDDEN)
                    },
                    error: if success {
                        None
                    } else {
                        Some("Authentication challenge failed.")
                    },
                    completed: &self.session.complete,
                    flows: &CONFIG.interactive_auth_flows,
                    params: &CONFIG.auth_params,
                    session: &jwt::encode(
                        &CONFIG.jwt_config.jwt_header,
                        &Claims::session(
                            &self.session.sub,
                            &self.session.device_id,
                            &self.session.complete,
                        ),
                        &CONFIG.jwt_config.auth_key,
                    )
                    .unknown()?,
                })
                .into())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum LoginType {
    #[serde(rename = "m.login.password")]
    Password,
    #[serde(rename = "m.login.token")]
    Token,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct LoginFlow {
    #[serde(rename = "type")]
    pub login_type: LoginType,
}
impl Borrow<LoginType> for LoginFlow {
    fn borrow(&self) -> &LoginType {
        &self.login_type
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "medium")]
#[serde(rename_all = "lowercase")]
pub enum ThirdParty {
    Email { address: String },
    MSISDN { address: String },
}

fn uid_optional_domain<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<UserId, D::Error> {
    let mut s: String = serde::Deserialize::deserialize(deserializer)?;
    if !s.contains(':') {
        s.reserve(1 + CONFIG.hostname.len());
        s.push(':');
        s.push_str(CONFIG.hostname.as_str());
    }
    UserId::try_from(s.as_str()).map_err(|_| {
        serde::de::Error::invalid_value(
            serde::de::Unexpected::Str(&s),
            &"a Matrix user ID as a string",
        )
    })
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
pub enum UserIdentifier {
    #[serde(rename = "m.id.user")]
    UserId {
        #[serde(deserialize_with = "uid_optional_domain")]
        user: UserId,
    },
    #[serde(rename = "m.id.thirdparty")]
    ThirdParty(ThirdParty),
    #[serde(rename = "m.id.phone")]
    PhoneNumber {
        country: String, // Alpha2
        phone: String,
    },
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
pub enum Challenge {
    #[serde(rename = "m.login.password")]
    Password { password: String },
    #[serde(rename = "m.login.token")]
    Token { token: String },
}
impl Challenge {
    pub async fn passes<T: Store>(
        &self,
        store: &T,
        user_id: &UserId,
        _device_id: &DeviceId,
    ) -> Result<Option<LoginType>, Error> {
        match self {
            Challenge::Password { password } => {
                let pwhash = store.fetch_password_hash(user_id).await?;
                if pwhash.matches(&password) {
                    Ok(Some(LoginType::Password))
                } else {
                    Ok(None)
                }
            }
            Challenge::Token { token } => {
                if store.check_otp_exists(&user_id, token).await? {
                    Ok(Some(LoginType::Token))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct LoginRequest {
    #[serde(flatten)]
    pub challenge: Challenge,
    pub identifier: UserIdentifier,
    pub device_id: Option<DeviceId>,
    pub initial_device_display_name: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct HomeserverInfo {
    pub base_url: Cow<'static, str>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct DiscoveryInfo {
    #[serde(rename = "m.homeserver")]
    pub homeserver: HomeserverInfo,
    // TODO: identity server??
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LoginResponse<'a> {
    pub user_id: &'a UserId,
    pub access_token: &'a str,
    pub device_id: &'a DeviceId,
    pub well_known: DiscoveryInfo,
}

// TODO
pub enum PWHash {}
impl PWHash {
    pub fn matches(&self, pw: &str) -> bool {
        unimplemented!()
    }
}
