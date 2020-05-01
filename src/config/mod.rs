use crate::models::auth::{InteractiveLoginFlow, LoginFlow, LoginType};
use ::config::{Config as Settings, Environment, File};
use actix_web::dev::Url;
use anyhow::Context;
use jsonwebtoken as jwt;
use linear_map::set::LinearSet;
use linear_map::LinearMap;
use log::warn;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn setup_settings() -> Result<Settings, anyhow::Error> {
    let mut settings = Settings::default();

    let filename = std::env::var("MAELSTROM_CONFIG_FILE").unwrap_or("Settings.yml".to_string());
    settings
        .merge(File::with_name(&filename))
        .with_context(|| format!("Could not merge configuration from file {}", filename))?;

    settings
        .merge(Environment::new().prefix("MAELSTROM"))
        .with_context(|| "Could not merge configuration from environment")?;

    Ok(settings)
}

#[derive(Deserialize)]
pub struct RawConfig {
    /// The port and address to run the server on
    pub server_addr: String,
    /// The hostname of the server, used to construct user's id
    pub hostname: String,
    /// The base url of the server, used to advertise homeserver information
    pub base_url: String,
    /// Database URL (will distinquish between postgres, sqlite, sled)
    pub database_url: String,
    /// Path to a PEM encoded ES256 key for creating auth tokens
    pub auth_key_file: PathBuf,
    /// Duration in seconds that an auth token is valid for
    pub auth_token_expiration: u64,
    /// Duration in seconds that a session token is valid for
    pub session_expiration: u64,
}

impl TryInto<Config> for RawConfig {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<Config, anyhow::Error> {
        let jwt_config = JwtConfig::new(&self.hostname, self.auth_key_file.as_path())
            .with_context(|| "Could not load JWT configuration")?;

        Ok(Config {
            server_addr: self.server_addr,
            hostname: self.hostname,
            base_url: self.base_url,
            database_url: self.database_url,
            jwt_config,
            auth_token_expiration: Duration::from_secs(self.auth_token_expiration),
            session_expiration: Duration::from_secs(self.session_expiration),
            auth_flows: {
                let mut set = LinearSet::new();
                set.insert(LoginFlow {
                    login_type: LoginType::Password,
                });
                set.insert(LoginFlow {
                    login_type: LoginType::Token,
                });
                set
            },
            interactive_auth_flows: {
                let mut set = LinearSet::new();
                set.insert(InteractiveLoginFlow {
                    stages: vec![LoginType::Password],
                });
                set
            },
            auth_params: LinearMap::new(),
        })
    }
}

pub struct JwtConfig {
    /// ES256 private key for creating auth and session tokens
    pub auth_key: jwt::EncodingKey,
    /// ES256 public key for verifying auth and session tokens (derived from private key)
    pub auth_key_pub: jwt::DecodingKey<'static>,
    /// Verification rules for auth and session tokens
    pub jwt_validation: jwt::Validation,
    /// Header for auth and session tokens
    pub jwt_header: jwt::Header,
}

impl JwtConfig {
    pub fn new(hostname: &str, auth_key_file: &Path) -> Result<Self, anyhow::Error> {
        let (auth_key, auth_key_pub) = {
            use std::io::Read;
            let mut key_data = Vec::new();
            std::fs::File::open(auth_key_file)
                .with_context(|| "Error opening AUTH_KEY_FILE.")?
                .read_to_end(&mut key_data)
                .with_context(|| "Error reading AUTH_KEY_FILE.")?;

            crate::util::crypto::parse_keypair(&key_data).with_context(|| {
                "Error decoding AUTH_KEY_FILE contents as a PEM encoded ECDSA private key."
            })?
        };

        Ok(JwtConfig {
            auth_key,
            auth_key_pub,
            jwt_validation: jwt::Validation {
                algorithms: vec![jwt::Algorithm::ES256],
                aud: None,
                iss: Some(hostname.to_string()),
                leeway: 5,
                sub: None,
                validate_exp: true,
                validate_nbf: false,
            },
            jwt_header: jwt::Header::new(jwt::Algorithm::ES256),
        })
    }
}

pub struct Config {
    /// The port and address to run the server on
    pub server_addr: String,
    /// The hostname of the server, used to construct user's id
    pub hostname: String,
    /// The base url of the server, used to advertise homeserver information
    pub base_url: String,
    /// Database URL (will distinquish between postgres, sqlite, sled)
    pub database_url: String,
    /// JWT configuration
    pub jwt_config: JwtConfig,
    /// Duration in seconds that an auth token is valid for
    pub auth_token_expiration: Duration,
    /// Duration in seconds that a session token is valid for
    pub session_expiration: Duration,
    /// Login flows available for standard auth
    pub auth_flows: LinearSet<LoginFlow>,
    /// Login flows available for interactive auth
    pub interactive_auth_flows: LinearSet<InteractiveLoginFlow>,
    /// Extra params needed by the client for authentication flows
    pub auth_params: LinearMap<LoginType, serde_json::Value>,
}

impl Config {
    pub fn load() -> Result<Self, anyhow::Error> {
        let settings = setup_settings()?;
        let raw_config: RawConfig = settings.try_into()?;

        raw_config.try_into()
    }
}
