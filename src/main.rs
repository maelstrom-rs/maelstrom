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
    #[serde(default)]
    media: Option<MediaConfig>,
    #[serde(default)]
    cluster: Option<ClusterConfig>,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    bind_address: String,
    server_name: String,
    public_base_url: String,
    /// Username to grant admin on startup (e.g. "admin"). Created if absent.
    admin_user: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DatabaseConfig {
    endpoint: String,
    namespace: String,
    database: String,
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MediaConfig {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    region: String,
    #[serde(default)]
    max_age_days: u64,
    #[serde(default = "default_sweep_interval")]
    sweep_interval_secs: u64,
}

fn default_sweep_interval() -> u64 {
    3600
}

#[derive(Debug, Deserialize)]
struct ClusterConfig {
    /// UDP address for chitchat gossip (e.g. "0.0.0.0:7280")
    listen_addr: String,
    /// Seed nodes for cluster discovery (e.g. ["node2:7280", "node3:7280"])
    #[serde(default)]
    seed_nodes: Vec<String>,
    /// Cluster identifier — nodes with different IDs ignore each other.
    #[serde(default = "default_cluster_id")]
    cluster_id: String,
}

fn default_cluster_id() -> String {
    "maelstrom".to_string()
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,maelstrom=debug".parse().unwrap()),
        )
        .init();

    info!("Starting Maelstrom Matrix homeserver");

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

    // Bootstrap admin user from config if specified
    if let Some(admin_username) = &config.server.admin_user {
        use maelstrom_storage::traits::UserStore;
        if !storage.user_exists(admin_username).await.unwrap_or(true) {
            // Create the admin user (no password — set via admin API or login)
            let admin = maelstrom_storage::traits::UserRecord {
                localpart: admin_username.clone(),
                password_hash: None,
                is_admin: true,
                is_guest: false,
                is_deactivated: false,
                created_at: chrono::Utc::now(),
            };
            if storage.create_user(&admin).await.is_ok() {
                info!(username = %admin_username, "Created admin user from config");
            }
        } else {
            // User exists — ensure admin flag is set
            let _ = storage.set_admin(admin_username, true).await;
            info!(username = %admin_username, "Ensured admin flag on configured user");
        }
    }

    // Build notifier (in-process broadcast channels)
    let notifier = maelstrom_api::notify::LocalNotifier::new();

    // Initialize rate limiter
    maelstrom_api::middleware::rate_limit::init();
    info!("Rate limiter initialized (in-memory)");

    // Connect to media store (RustFS / S3) — optional
    let media_client = if let Some(ref media_conf) = config.media {
        let media_config = maelstrom_media::client::MediaConfig {
            endpoint: media_conf.endpoint.clone(),
            bucket: media_conf.bucket.clone(),
            access_key: media_conf.access_key.clone(),
            secret_key: media_conf.secret_key.clone(),
            region: media_conf.region.clone(),
        };

        match maelstrom_media::client::MediaClient::connect(&media_config).await {
            Ok(client) => {
                // Spawn media retention background task
                let retention_config = maelstrom_media::retention::RetentionConfig {
                    max_age_days: media_conf.max_age_days,
                    sweep_interval_secs: media_conf.sweep_interval_secs,
                    batch_size: 500,
                };

                let _retention_handle = maelstrom_media::retention::spawn_retention_task(
                    retention_config,
                    storage.clone(),
                    client.clone(),
                );

                info!("Connected to media store");
                Some(client)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Media store unavailable — media endpoints will return errors");
                None
            }
        }
    } else {
        info!("No [media] config — media endpoints disabled");
        None
    };

    // Initialize federation signing key
    let server_name = ServerName::new(&config.server.server_name);
    let signing_key = {
        use maelstrom_storage::traits::FederationKeyStore;
        let keys = storage.get_active_server_keys().await.unwrap_or_default();
        if let Some(key_record) = keys.first() {
            use base64::Engine;
            let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
            let private_bytes = engine.decode(&key_record.private_key)
                .context("Invalid stored private key")?;
            let key_bytes: [u8; 32] = private_bytes.try_into()
                .map_err(|_| anyhow::anyhow!("Invalid key length"))?;
            info!(key_id = %key_record.key_id, "Loaded existing signing key");
            maelstrom_core::signatures::keys::KeyPair::from_bytes(
                key_record.key_id.clone(),
                &key_bytes,
            )
        } else {
            let kp = maelstrom_core::signatures::keys::KeyPair::generate();
            use base64::Engine;
            let engine = base64::engine::general_purpose::STANDARD_NO_PAD;
            let record = maelstrom_storage::traits::ServerKeyRecord {
                key_id: kp.key_id().to_string(),
                algorithm: "ed25519".to_string(),
                public_key: kp.public_key_base64(),
                private_key: engine.encode(kp.private_key_bytes()),
                valid_until: chrono::Utc::now() + chrono::Duration::days(365),
            };
            storage.store_server_key(&record).await
                .context("Failed to store signing key")?;
            info!(key_id = %kp.key_id(), "Generated new signing key");
            kp
        }
    };

    // Build shared ephemeral store for typing/presence.
    // In cluster mode, wire up chitchat gossip for cross-node propagation.
    let notifier: std::sync::Arc<dyn maelstrom_api::notify::Notifier> =
        std::sync::Arc::new(notifier);

    let (ephemeral, _gossip_bridge) = if let Some(ref cluster) = config.cluster {
        use std::time::Duration;

        let listen_addr: std::net::SocketAddr = cluster
            .listen_addr
            .parse()
            .context("Invalid cluster.listen_addr")?;

        // generation_id is a monotonic restart counter — epoch seconds works.
        let generation = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let node_id = chitchat::ChitchatId::new(
            config.server.server_name.clone(),
            generation,
            listen_addr,
        );

        let chitchat_config = chitchat::ChitchatConfig {
            chitchat_id: node_id,
            cluster_id: cluster.cluster_id.clone(),
            gossip_interval: Duration::from_millis(500),
            listen_addr,
            seed_nodes: cluster.seed_nodes.clone(),
            failure_detector_config: Default::default(),
            marked_for_deletion_grace_period: Duration::from_secs(60),
            catchup_callback: None,
            extra_liveness_predicate: None,
        };

        let (store, delta_rx) = maelstrom_core::ephemeral::EphemeralStore::with_gossip();
        let ephemeral = std::sync::Arc::new(store);

        let chitchat_handle = chitchat::spawn_chitchat(
            chitchat_config,
            vec![],
            &chitchat::transport::UdpTransport,
        )
        .await
        .context("Failed to start chitchat gossip")?;

        let bridge = maelstrom_api::gossip::start(
            &chitchat_handle,
            ephemeral.clone(),
            notifier.clone(),
            delta_rx,
        )
        .await;

        info!(
            listen = %cluster.listen_addr,
            seeds = ?cluster.seed_nodes,
            "Cluster mode: chitchat gossip started"
        );

        (ephemeral, Some((chitchat_handle, bridge)))
    } else {
        let ephemeral =
            std::sync::Arc::new(maelstrom_core::ephemeral::EphemeralStore::new());
        info!("Single-node mode (no [cluster] config)");
        (ephemeral, None)
    };

    // Build federation state and router
    let federation_state = maelstrom_federation::FederationState::new(
        storage.clone(),
        ephemeral.clone(),
        signing_key,
        server_name.clone(),
    );
    let federation_router = maelstrom_federation::router::build(federation_state);

    // Build admin state and router (with retention config for management)
    let admin_retention = config.media.as_ref().map(|m| maelstrom_admin::RetentionConfig {
        max_age_days: m.max_age_days,
        sweep_interval_secs: m.sweep_interval_secs,
        batch_size: 500,
    }).unwrap_or_default();
    let admin_state = maelstrom_admin::AdminState::with_retention(
        storage.clone(),
        server_name.clone(),
        admin_retention,
    );
    let admin_router = maelstrom_admin::router::build(admin_state);

    // Build application state
    let state = if let Some(mc) = media_client {
        maelstrom_api::state::AppState::with_media(
            storage,
            notifier,
            ephemeral,
            mc,
            server_name,
            config.server.public_base_url,
        )
    } else {
        maelstrom_api::state::AppState::new(
            storage,
            notifier,
            ephemeral,
            server_name,
            config.server.public_base_url,
        )
    };

    let app = maelstrom_api::router::build(state)
        .merge(federation_router)
        .merge(admin_router);

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
