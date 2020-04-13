use std::borrow::Cow;

use ruma_identifiers::{DeviceId, UserId};

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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type")]
pub enum UserIdentifier {
    #[serde(rename = "m.id.user")]
    UserId { user: UserId },
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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct LoginResponse<'a, 'b> {
    pub user_id: Cow<'a, UserId>,
    pub access_token: String,
    pub device_id: Cow<'b, DeviceId>,
    pub well_known: DiscoveryInfo,
}
