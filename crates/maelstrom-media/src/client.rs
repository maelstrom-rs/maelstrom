use aws_sdk_s3::Client;
use tracing::info;

/// Configuration for the S3-compatible media store (RustFS).
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// S3 endpoint URL, e.g. `http://localhost:9000`
    pub endpoint: String,
    /// Bucket name for media storage
    pub bucket: String,
    /// Access key
    pub access_key: String,
    /// Secret key
    pub secret_key: String,
    /// AWS region (required by SDK, use `us-east-1` for S3-compatible stores)
    pub region: String,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:9000".to_string(),
            bucket: "maelstrom-media".to_string(),
            access_key: "maelstrom".to_string(),
            secret_key: "maelstrom".to_string(),
            region: "us-east-1".to_string(),
        }
    }
}

/// S3 client wrapper for media operations.
#[derive(Clone)]
pub struct MediaClient {
    client: Client,
    bucket: String,
}

impl MediaClient {
    /// Create a new MediaClient connected to the S3-compatible store.
    pub async fn connect(config: &MediaConfig) -> Result<Self, MediaError> {
        info!(endpoint = %config.endpoint, bucket = %config.bucket, "Connecting to media store");

        let creds = aws_sdk_s3::config::Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "maelstrom",
        );

        let s3_config = aws_sdk_s3::Config::builder()
            .endpoint_url(&config.endpoint)
            .region(aws_sdk_s3::config::Region::new(config.region.clone()))
            .credentials_provider(creds)
            .force_path_style(true)
            .behavior_version_latest()
            .build();

        let client = Client::from_conf(s3_config);

        // Ensure bucket exists
        let buckets = client
            .list_buckets()
            .send()
            .await
            .map_err(|e| MediaError::Connection(e.to_string()))?;

        let bucket_exists = buckets
            .buckets()
            .iter()
            .any(|b| b.name() == Some(&config.bucket));

        if !bucket_exists {
            client
                .create_bucket()
                .bucket(&config.bucket)
                .send()
                .await
                .map_err(|e| MediaError::Connection(format!("Failed to create bucket: {e}")))?;
            info!(bucket = %config.bucket, "Created media bucket");
        }

        info!("Connected to media store");

        Ok(Self {
            client,
            bucket: config.bucket.clone(),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Check if the media store is reachable.
    pub async fn is_healthy(&self) -> bool {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Upload failed: {0}")]
    Upload(String),

    #[error("Download failed: {0}")]
    Download(String),

    #[error("Not found: {0}")]
    NotFound(String),
}
