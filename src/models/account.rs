use super::auth::AuthToken;
use ruma_identifiers::UserId;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct WhoamiRequest {
    pub access_token: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WhoamiResponse {
    pub user_id: UserId,
}
