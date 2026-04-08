use chrono::Utc;
use maelstrom_storage::traits::Storage;
use tracing::{debug, error, info, warn};

use crate::client::MediaClient;

/// Configuration for the media retention policy.
#[derive(Debug, Clone)]
pub struct RetentionConfig {
    /// Maximum age of media in days. Media older than this will be purged.
    /// Set to 0 to disable age-based retention.
    pub max_age_days: u64,
    /// How often to run the retention sweep, in seconds.
    pub sweep_interval_secs: u64,
    /// Maximum number of items to delete per sweep (prevents long-running deletes).
    pub batch_size: usize,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            max_age_days: 0,           // disabled by default
            sweep_interval_secs: 3600, // every hour
            batch_size: 500,
        }
    }
}

/// Spawn the retention background task.
///
/// This runs in a loop, periodically checking for media older than `max_age_days`
/// and deleting both the S3 object and the metadata record.
///
/// The task runs until the provided cancellation token is dropped (or the process exits).
pub fn spawn_retention_task(
    config: RetentionConfig,
    storage: impl Storage + 'static,
    media_client: MediaClient,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if config.max_age_days == 0 {
            info!("Media retention disabled (max_age_days = 0)");
            return;
        }

        info!(
            max_age_days = config.max_age_days,
            sweep_interval_secs = config.sweep_interval_secs,
            batch_size = config.batch_size,
            "Media retention task started"
        );

        let interval = std::time::Duration::from_secs(config.sweep_interval_secs);

        loop {
            tokio::time::sleep(interval).await;

            let cutoff = Utc::now() - chrono::Duration::days(config.max_age_days as i64);

            debug!(cutoff = %cutoff, "Running retention sweep");

            match storage.list_media_before(cutoff, config.batch_size).await {
                Ok(expired) => {
                    if expired.is_empty() {
                        debug!("No expired media found");
                        continue;
                    }

                    info!(count = expired.len(), "Found expired media to purge");

                    for record in &expired {
                        // Delete from S3 first
                        if let Err(e) = media_client.delete(&record.s3_key).await {
                            warn!(
                                media_id = %record.media_id,
                                s3_key = %record.s3_key,
                                error = %e,
                                "Failed to delete media from S3, skipping"
                            );
                            continue;
                        }

                        // Then delete metadata
                        if let Err(e) = storage
                            .delete_media(&record.server_name, &record.media_id)
                            .await
                        {
                            error!(
                                media_id = %record.media_id,
                                error = %e,
                                "Failed to delete media metadata after S3 deletion"
                            );
                        } else {
                            debug!(
                                media_id = %record.media_id,
                                "Purged expired media"
                            );
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to list expired media");
                }
            }
        }
    })
}
