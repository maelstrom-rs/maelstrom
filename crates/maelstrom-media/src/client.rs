//! S3-compatible object storage client for media content.
//!
//! [`MediaClient`] wraps the AWS S3 SDK (`aws-sdk-s3`) to provide a simple
//! async interface for media CRUD operations against any S3-compatible store
//! (RustFS, AWS S3, etc.).
//!
//! ## Connection
//!
//! [`MediaClient::connect`] builds an S3 client from a [`MediaConfig`]
//! (endpoint URL, bucket name, credentials, region) with `force_path_style`
//! enabled (required for non-AWS S3-compatible stores). On first connect it
//! checks whether the configured bucket exists and creates it if missing.
//!
//! ## Operations
//!
//! - [`MediaClient::upload`]   -- `PutObject` with content-type metadata.
//! - [`MediaClient::download`] -- `GetObject`, returning bytes + content-type + length.
//! - [`MediaClient::delete`]   -- `DeleteObject` by key.
//! - [`MediaClient::exists`]   -- `HeadObject` existence check.
//! - [`MediaClient::is_healthy`] -- `HeadBucket` liveness probe.
//!
//! ## Error handling
//!
//! All fallible operations return [`MediaError`], which distinguishes connection
//! failures, upload/download errors, and not-found conditions so that callers
//! can map them to the appropriate Matrix error codes.

use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use tracing::{debug, info};

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

    /// Upload bytes to S3 with the given key and content type.
    pub async fn upload(
        &self,
        key: &str,
        data: Bytes,
        content_type: &str,
    ) -> Result<(), MediaError> {
        debug!(key = %key, content_type = %content_type, size = data.len(), "Uploading to S3");

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(data))
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| MediaError::Upload(e.to_string()))?;

        Ok(())
    }

    /// Download bytes from S3 by key.
    pub async fn download(&self, key: &str) -> Result<DownloadResult, MediaError> {
        let output = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("NoSuchKey") || msg.contains("not found") {
                    MediaError::NotFound(key.to_string())
                } else {
                    MediaError::Download(msg)
                }
            })?;

        let content_type = output
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        let content_length = output.content_length().unwrap_or(0) as u64;

        let data = output
            .body
            .collect()
            .await
            .map_err(|e| MediaError::Download(e.to_string()))?
            .into_bytes();

        Ok(DownloadResult {
            data,
            content_type,
            content_length,
        })
    }

    /// Delete an object from S3 by key.
    pub async fn delete(&self, key: &str) -> Result<(), MediaError> {
        debug!(key = %key, "Deleting from S3");

        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|e| MediaError::Upload(e.to_string()))?;

        Ok(())
    }

    /// Check if an object exists in S3.
    pub async fn exists(&self, key: &str) -> Result<bool, MediaError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("NotFound") || msg.contains("404") || msg.contains("not found") {
                    Ok(false)
                } else {
                    Err(MediaError::Download(msg))
                }
            }
        }
    }
}

/// Result of downloading a file from S3.
pub struct DownloadResult {
    pub data: Bytes,
    pub content_type: String,
    pub content_length: u64,
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
