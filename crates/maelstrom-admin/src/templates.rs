//! Askama HTML templates for the admin SSR dashboard.
//!
//! Each struct in this module derives [`askama::Template`] and maps to an HTML
//! file under `templates/pages/`. Askama compiles these templates at build time
//! into Rust code, so rendering is allocation-light and cannot fail at runtime
//! due to missing template files.
//!
//! Templates are rendered in the [`super::handlers::dashboard`] handlers and
//! returned as `Html<String>` responses. The template files use Jinja2-style
//! syntax with access to the struct fields for dynamic content (server name,
//! uptime, memory usage, etc.).

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
