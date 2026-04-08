use std::collections::HashMap;
use std::sync::Mutex;

use maelstrom_core::identifiers::ServerName;
use maelstrom_core::signatures::keys::KeyPair;
use tracing::debug;

/// Outbound federation HTTP client with server discovery and request signing.
pub struct FederationClient {
    http: reqwest::Client,
    signing_key: KeyPair,
    server_name: ServerName,
    /// Cache of server_name -> resolved endpoint URL.
    endpoints: Mutex<HashMap<String, String>>,
}

impl FederationClient {
    pub fn new(signing_key: KeyPair, server_name: ServerName) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Maelstrom Matrix Homeserver")
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            signing_key,
            server_name,
            endpoints: Mutex::new(HashMap::new()),
        }
    }

    /// Discover the federation endpoint for a server.
    ///
    /// Tries `.well-known/matrix/server` first, falls back to `server_name:8448`.
    pub async fn discover(&self, server_name: &str) -> String {
        // Check cache
        {
            let cache = self.endpoints.lock().unwrap();
            if let Some(url) = cache.get(server_name) {
                return url.clone();
            }
        }

        let endpoint = self.do_discover(server_name).await;

        // Cache
        let mut cache = self.endpoints.lock().unwrap();
        cache.insert(server_name.to_string(), endpoint.clone());
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
                        && let Some(server) = body.get("m.server").and_then(|s| s.as_str()) {
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

        serde_json::from_str(&text)
            .map_err(|e| FederationError::InvalidResponse(e.to_string()))
    }

    /// Send a signed PUT request with a JSON body to a remote server.
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

        let body_str = serde_json::to_string(body)
            .map_err(|e| FederationError::Request(e.to_string()))?;

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

        serde_json::from_str(&text)
            .map_err(|e| FederationError::InvalidResponse(e.to_string()))
    }

    /// Fetch a remote server's signing keys.
    pub async fn fetch_server_keys(
        &self,
        server_name: &str,
    ) -> Result<serde_json::Value, FederationError> {
        self.get(server_name, "/_matrix/key/v2/server").await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    #[error("Request failed: {0}")]
    Request(String),

    #[error("Remote error: {0}")]
    Remote(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}
