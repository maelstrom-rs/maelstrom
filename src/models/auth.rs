use std::borrow::Cow;
use std::convert::TryFrom;

use ruma_identifiers::{DeviceId, UserId};

use crate::CONFIG;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
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
