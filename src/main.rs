use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::info;

use maelstrom_core::identifiers::ServerName;
use maelstrom_storage::surreal::connection::SurrealConfig;

/// Top-level configuration, deserialized from TOML.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Config {
    server: ServerConfig,
    database: DatabaseConfig,
    media: MediaConfig,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    /// Address to bind to, e.g. `0.0.0.0:8008`
    bind_address: String,
    /// Public-facing server name, e.g. `example.com`
    server_name: String,
    /// Public base URL, e.g. `https://matrix.example.com`
    public_base_url: String,
}

#[derive(Debug, Deserialize)]
struct DatabaseConfig {
    /// SurrealDB endpoint, e.g. `ws://localhost:8000` or `mem://`
    endpoint: String,
    /// Namespace
    namespace: String,
    /// Database
    database: String,
    /// Username
    username: String,
    /// Password
    password: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MediaConfig {
    /// S3 endpoint URL
    endpoint: String,
    /// Bucket name
    bucket: String,
    /// Access key
    access_key: String,
    /// Secret key
    secret_key: String,
    /// Region
    region: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,maelstrom=debug".parse().unwrap()),
        )
        .init();

    info!("Starting Maelstrom Matrix homeserver");

    // Load configuration
    let config = load_config().context("Failed to load configuration")?;

    info!(
        server_name = %config.server.server_name,
        bind_address = %config.server.bind_address,
        "Configuration loaded"
    );

    // Connect to SurrealDB
    let surreal_config = SurrealConfig {
        endpoint: config.database.endpoint,
        namespace: config.database.namespace,
        database: config.database.database,
        username: config.database.username,
        password: config.database.password,
    };

    let storage = maelstrom_storage::SurrealStorage::connect(&surreal_config)
        .await
        .context("Failed to connect to SurrealDB")?;

    // Build application state
    let state = maelstrom_api::state::AppState::new(
        storage,
        ServerName::new(&config.server.server_name),
        config.server.public_base_url,
    );

    // Build router
    let app = maelstrom_api::router::build(state);

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.server.bind_address)
        .await
        .context("Failed to bind to address")?;

    info!(
        address = %config.server.bind_address,
        "Listening for connections"
    );

    axum::serve(listener, app)
        .await
        .context("Server error")?;

    Ok(())
}

fn load_config() -> Result<Config> {
    let config_path = std::env::var("MAELSTROM_CONFIG")
        .unwrap_or_else(|_| "config/local.toml".to_string());

    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {config_path}"))?;

    let config: Config =
        toml::from_str(&content).with_context(|| format!("Failed to parse config: {config_path}"))?;

    Ok(config)
}
