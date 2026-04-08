use askama::Template;

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
pub struct DashboardPage {
    pub server_name: String,
    pub version: String,
    pub uptime: String,
    pub db_status: &'static str,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
}

#[derive(Template)]
#[template(path = "pages/users.html")]
pub struct UsersPage {}

#[derive(Template)]
#[template(path = "pages/rooms.html")]
pub struct RoomsPage {}

#[derive(Template)]
#[template(path = "pages/federation.html")]
pub struct FederationPage {
    pub server_name: String,
    pub signing_key_count: usize,
}
