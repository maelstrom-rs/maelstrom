//! # Outbound Federation HTTP Client
//!
//! This module provides [`FederationClient`], the HTTP client used for all outbound
//! federation requests -- any time this server needs to talk to another Matrix server.
//!
//! ## Server Discovery
//!
//! Before sending a request, the client must figure out where the destination server
//! actually lives. Matrix defines a multi-step discovery process:
//!
//! 1. **Explicit port** -- if the server name already contains a port (e.g.,
//!    `matrix.example.com:443`), use it directly.
//! 2. **`.well-known`** -- try `GET https://{server_name}/.well-known/matrix/server`.
//!    If it returns `{"m.server": "delegated.example.com:8448"}`, use that.
//! 3. **SRV DNS lookup** -- query `_matrix-fed._tcp.{server_name}` for an SRV record
//!    that points to the actual host and port.
//! 4. **Port 8448 fallback** -- if nothing else works, try `https://{server_name}:8448`.
//!
//! Resolved endpoints are cached in a [`DashMap`] so discovery only happens once per
//! destination (until the process restarts).
//!
//! ## Request Signing
//!
//! Every outbound federation request is signed using the **X-Matrix** authorization
//! scheme. The client uses [`sign_request`](crate::signing::sign_request) to produce
//! an `Authorization` header that includes the origin server name, destination, key ID,
//! and a signature over the canonical JSON representation of the request.
//!
//! ## Error Handling
//!
//! Federation requests can fail in three ways, represented by [`FederationError`]:
//!
//! - **`Request`** -- network-level failure (DNS, TLS, timeout)
//! - **`Remote`** -- the remote server returned an HTTP error (4xx, 5xx)
//! - **`InvalidResponse`** -- the response body was not valid JSON

use dashmap::DashMap;
use maelstrom_core::matrix::id::ServerName;
use maelstrom_core::matrix::keys::KeyPair;
use tracing::debug;

/// Outbound federation HTTP client with server discovery and request signing.
///
/// Wraps a [`reqwest::Client`] with Matrix-specific functionality: automatic server
/// discovery (`.well-known` / SRV / port 8448 fallback), endpoint caching via
/// [`DashMap`], and X-Matrix request signing on every outbound request.
///
/// # TLS Configuration
///
/// In production, a custom CA certificate can be provided for TLS verification.
/// When no CA path is given, the client falls back to accepting all certificates
/// (development mode only).
pub struct FederationClient {
    http: reqwest::Client,
    signing_key: KeyPair,
    server_name: ServerName,
    /// Cache of server_name -> resolved endpoint URL.
    endpoints: DashMap<String, String>,
}

impl FederationClient {
    pub fn new(signing_key: KeyPair, server_name: ServerName) -> Self {
        Self::with_ca(signing_key, server_name, None)
    }

    /// Create a federation client, optionally trusting a specific CA certificate.
    /// When `ca_path` is provided, the client trusts that CA for TLS verification.
    /// When absent, falls back to accepting all certificates (dev mode).
    pub fn with_ca(signing_key: KeyPair, server_name: ServerName, ca_path: Option<&str>) -> Self {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Maelstrom Matrix Homeserver");

        if let Some(path) = ca_path {
            if let Ok(pem) = std::fs::read(path) {
                if let Ok(cert) = reqwest::Certificate::from_pem(&pem) {
                    debug!(path = %path, "Loaded custom CA certificate for federation");
                    builder = builder.add_root_certificate(cert);
                } else {
                    tracing::warn!(path = %path, "Failed to parse CA certificate, falling back to insecure");
                    builder = builder.danger_accept_invalid_certs(true);
                }
            } else {
                tracing::warn!(path = %path, "Failed to read CA certificate, falling back to insecure");
                builder = builder.danger_accept_invalid_certs(true);
            }
        } else {
            builder = builder.danger_accept_invalid_certs(true);
        }

        let http = builder.build().expect("Failed to build HTTP client");

        Self {
            http,
            signing_key,
            server_name,
            endpoints: DashMap::new(),
        }
    }

    /// Discover the federation endpoint for a server.
    ///
    /// Tries `.well-known/matrix/server` first, falls back to `server_name:8448`.
    pub async fn discover(&self, server_name: &str) -> String {
        // Check cache
        if let Some(url) = self.endpoints.get(server_name) {
            return url.clone();
        }

        let endpoint = self.do_discover(server_name).await;

        // Cache
        self.endpoints
            .insert(server_name.to_string(), endpoint.clone());
        endpoint
    }

    async fn do_discover(&self, server_name: &str) -> String {
        // If server_name already has a port, use it directly
        if server_name.contains(':') {
            return format!("https://{server_name}");
        }

        // Try .well-known
        let well_known_url = format!("https://{server_name}/.well-known/matrix/server");
        debug!(url = %well_known_url, "Trying .well-known discovery");

        if let Ok(resp) = self.http.get(&well_known_url).send().await
            && resp.status().is_success()
            && let Ok(text) = resp.text().await
            && let Ok(body) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(server) = body.get("m.server").and_then(|s| s.as_str())
        {
            debug!(server = %server, "Discovered via .well-known");
            if server.contains(':') {
                return format!("https://{server}");
            } else {
                return format!("https://{server}:8448");
            }
        }

        // Try SRV record: _matrix-fed._tcp.{server_name}
        if let Some(endpoint) = self.try_srv_lookup(server_name).await {
            debug!(endpoint = %endpoint, "Discovered via SRV record");
            return endpoint;
        }

        // Fallback to port 8448
        debug!(server_name = %server_name, "Falling back to port 8448");
        format!("https://{server_name}:8448")
    }

    /// Try SRV DNS lookup for `_matrix-fed._tcp.{server_name}`.
    ///
    /// Uses a system DNS query via tokio's spawn_blocking + std::process.
    async fn try_srv_lookup(&self, server_name: &str) -> Option<String> {
        let srv_name = format!("_matrix-fed._tcp.{server_name}");

        tokio::task::spawn_blocking(move || {
            // Use the `dig` command for SRV lookup (available on most Unix systems)
            let output = std::process::Command::new("dig")
                .args(["+short", "SRV", &srv_name])
                .output()
                .ok()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Parse first SRV line: "priority weight port target"
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let port = parts[2];
                    let host = parts[3].trim_end_matches('.');
                    if !host.is_empty() && host != "." {
                        return Some(format!("https://{host}:{port}"));
                    }
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }

    /// Send a signed GET request to a remote server.
    ///
    /// Performs server discovery, signs the request with the X-Matrix scheme,
    /// and returns the parsed JSON response. Returns a [`FederationError`] if
    /// discovery fails, the remote returns an error, or the response is not valid JSON.
    pub async fn get(
        &self,
        destination: &str,
        path: &str,
    ) -> Result<serde_json::Value, FederationError> {
        let base_url = self.discover(destination).await;
        let url = format!("{base_url}{path}");

        let auth = crate::signing::sign_request(
            &self.signing_key,
            self.server_name.as_str(),
            destination,
            "GET",
            path,
            None,
        );

        let response = self
            .http
            .get(&url)
            .header("Authorization", auth)
            .send()
            .await
            .map_err(|e| FederationError::Request(format!("{destination}: {e}")))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| FederationError::Request(e.to_string()))?;

        if !status.is_success() {
            return Err(FederationError::Remote(format!(
                "{destination} returned {}: {text}",
                status.as_u16()
            )));
        }

        serde_json::from_str(&text).map_err(|e| FederationError::InvalidResponse(e.to_string()))
    }

    /// Send a signed PUT request with a JSON body to a remote server.
    ///
    /// Used for sending federation transactions (`/send/{txnId}`), join events
    /// (`/send_join`), and other write operations. The body is included in the
    /// X-Matrix signature computation.
    pub async fn put_json(
        &self,
        destination: &str,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, FederationError> {
        let base_url = self.discover(destination).await;
        let url = format!("{base_url}{path}");

        let auth = crate::signing::sign_request(
            &self.signing_key,
            self.server_name.as_str(),
            destination,
            "PUT",
            path,
            Some(body),
        );

        let body_str =
            serde_json::to_string(body).map_err(|e| FederationError::Request(e.to_string()))?;

        let response = self
            .http
            .put(&url)
            .header("Authorization", auth)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
            .await
            .map_err(|e| FederationError::Request(format!("{destination}: {e}")))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| FederationError::Request(e.to_string()))?;

        if !status.is_success() {
            return Err(FederationError::Remote(format!(
                "{destination} returned {}: {text}",
                status.as_u16()
            )));
        }

        serde_json::from_str(&text).map_err(|e| FederationError::InvalidResponse(e.to_string()))
    }

    /// Fetch a remote server's signing keys from `/_matrix/key/v2/server`.
    ///
    /// Every Matrix homeserver publishes its Ed25519 signing keys at this well-known
    /// endpoint. The returned JSON includes `verify_keys`, `old_verify_keys`, and
    /// `valid_until_ts`.
    pub async fn fetch_server_keys(
        &self,
        server_name: &str,
    ) -> Result<serde_json::Value, FederationError> {
        self.get(server_name, "/_matrix/key/v2/server").await
    }
}

/// Errors that can occur during outbound federation requests.
///
/// These cover the three failure modes of talking to a remote server:
/// network issues, remote HTTP errors, and unparseable responses.
#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    /// Network-level failure: DNS resolution, TLS handshake, connection timeout, etc.
    #[error("Request failed: {0}")]
    Request(String),

    /// The remote server returned an HTTP error status (4xx or 5xx).
    #[error("Remote error: {0}")]
    Remote(String),

    /// The response body could not be parsed as valid JSON.
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}
