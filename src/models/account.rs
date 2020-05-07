use ruma_identifiers::UserId;

#[derive(Clone, Debug, serde::Serialize)]
pub struct WhoamiResponse {
    pub user_id: UserId,
}
