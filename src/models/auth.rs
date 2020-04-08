use std::borrow::Cow;

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

#[derive(Clone, Debug)]
pub struct UserId {
    pub local_part: String,
    pub domain: Cow<'static, str>,
}
impl<'de> serde::Deserialize<'de> for UserId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut str_id: String = serde::Deserialize::deserialize(deserializer)?;
        if let Some(colon_idx) = str_id.find(":") {
            let domain = str_id.split_off(colon_idx + 1);
            str_id.truncate(str_id.len() - 1);
            Ok(UserId {
                local_part: str_id,
                domain: Cow::Owned(domain),
            })
        } else {
            Ok(UserId {
                local_part: str_id,
                domain: Cow::Borrowed(&CONFIG.hostname),
            })
        }
    }
}
impl serde::Serialize for UserId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{}:{}", self.local_part, self.domain))
    }
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
    pub device_id: Option<String>,
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
pub struct LoginResponse {
    pub user_id: UserId,
    pub access_token: String,
    pub device_id: String,
    pub well_known: DiscoveryInfo,
}
