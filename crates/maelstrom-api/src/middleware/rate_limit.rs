use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::debug;

/// Sliding-window rate limiter (in-memory, per-node).
///
/// For multi-node deployments, each node enforces its own limit.
/// A shared rate-limit layer can be added later via SurrealDB or
/// a lightweight distributed counter without an external broker.
#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimitState>>,
    max_requests: u32,
    window_secs: u64,
}

struct RateLimitState {
    counters: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimitState {
                counters: HashMap::new(),
            })),
            max_requests,
            window_secs,
        }
    }

    /// Check if a request from the given key should be allowed.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);
        let mut state = self.state.lock().unwrap();

        let timestamps = state.counters.entry(key.to_string()).or_default();

        // Remove expired entries
        timestamps.retain(|t| now.duration_since(*t) < window);

        if timestamps.len() >= self.max_requests as usize {
            false
        } else {
            timestamps.push(now);
            true
        }
    }

    /// Periodic cleanup of stale entries.
    pub fn cleanup(&self) {
        let now = Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);
        let mut state = self.state.lock().unwrap();

        state.counters.retain(|_, timestamps| {
            timestamps.retain(|t| now.duration_since(*t) < window);
            !timestamps.is_empty()
        });
    }
}

/// Global rate limiter instance. Initialized at startup.
static GLOBAL_LIMITER: std::sync::LazyLock<Mutex<Option<RateLimiter>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Initialize the global rate limiter. Call once at startup.
pub fn init() {
    *GLOBAL_LIMITER.lock().unwrap() = Some(RateLimiter::new(100, 60));
}

fn get_limiter() -> RateLimiter {
    GLOBAL_LIMITER
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| RateLimiter::new(100, 60))
}

/// Axum middleware layer for rate limiting login/register endpoints.
pub async fn rate_limit_auth(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path();
    let is_auth_endpoint = path.contains("/login") || path.contains("/register");

    if !is_auth_endpoint {
        return next.run(request).await;
    }

    let ip = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or("unknown").trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let limiter = get_limiter();

    if !limiter.check(&ip) {
        debug!(ip = %ip, "Rate limited auth request");

        let body = serde_json::json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "Too many requests",
            "retry_after_ms": 5000,
        });

        return (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
    }

    next.run(request).await
}
