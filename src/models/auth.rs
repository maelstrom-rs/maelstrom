use std::borrow::Cow;
use std::collections::VecDeque;
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

use jsonwebtoken as jwt;
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
    pub iat: i64,
    pub exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<u32>,
    pub sub: &'a UserId,
    pub device_id: &'a DeviceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<&'a [LoginType]>,
}
impl<'a> Claims<'a> {
    pub fn auth(user_id: &'a UserId, device_id: &'a DeviceId) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|a| a.as_secs() as i64)
            .unwrap_or_else(|a| -(a.duration().as_secs() as i64));
        Self {
            kind: TokenKind::Auth,
            iss: &CONFIG.hostname,
            iat: now,
            exp: now + CONFIG.auth_token_expiration,
            jti: Some(rand::random()),
            sub: user_id,
            device_id,
            incomplete: None,
        }
    }

    pub fn session(user_id: &'a UserId, device_id: &'a DeviceId, flows: &'a [LoginType]) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|a| a.as_secs() as i64)
            .unwrap_or_else(|a| -(a.duration().as_secs() as i64));
        Self {
            kind: TokenKind::Session,
            iss: &CONFIG.hostname,
            iat: now,
            exp: now + CONFIG.session_expiration,
            jti: None,
            sub: user_id,
            device_id,
            incomplete: Some(flows),
        }
    }

    pub fn as_jwt(&self) -> Result<String, jwt::errors::Error> {
        jwt::encode(
            &jwt::Header::new(jwt::Algorithm::ES256),
            &self,
            &CONFIG.auth_key,
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionToken {
    pub sub: UserId,
    pub device_id: DeviceId,
    pub incomplete: VecDeque<LoginType>,
}
impl FromStr for SessionToken {
    type Err = jwt::errors::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let data = jwt::decode(s, &CONFIG.auth_key_pub, &CONFIG.jwt_validation)?;
        Ok(data.claims)
    }
}
impl SessionToken {
    pub async fn update<T: Store>(
        &mut self,
        store: &T,
        challenge: &Challenge,
    ) -> Result<(), Error> {
        if challenge
            .passes(
                store,
                self.incomplete.get(0).copied(),
                &self.sub,
                &self.device_id,
            )
            .await?
        {
            self.incomplete.pop_front();
        }
        Ok(())
    }

    pub fn complete(&self) -> bool {
        self.incomplete.is_empty()
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
        let data = jwt::decode(s, &CONFIG.auth_key_pub, &CONFIG.jwt_validation)?;
        Ok(data.claims)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct InteractiveAuth {
    #[serde(flatten)]
    pub challenge: Challenge,
    #[serde(deserialize_with = "crate::util::serde::deser_parse")]
    pub session: SessionToken,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
pub enum LoginType {
    #[serde(rename = "m.login.password")]
    Password,
    #[serde(rename = "m.login.token")]
    Token,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct LoginFlow {
    #[serde(rename = "type")]
    pub login_type: LoginType,
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
        login_type: Option<LoginType>,
        user_id: &UserId,
        _device_id: &DeviceId,
    ) -> Result<bool, Error> {
        match (self, login_type) {
            (Challenge::Password { password }, Some(LoginType::Password))
            | (Challenge::Password { password }, None) => {
                let pwhash = store.fetch_password_hash(user_id).await?;
                Ok(pwhash.matches(&password))
            }
            (Challenge::Token { token }, Some(LoginType::Token))
            | (Challenge::Token { token }, None) => {
                Ok(store.check_otp_exists(&user_id, token).await?)
            }
            _ => Ok(false),
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
