//! Media upload, download, thumbnails, and URL previews.
//!
//! Media in Matrix is referenced by **MXC URIs** of the form
//! `mxc://server_name/media_id`. The upload flow works as follows:
//!
//! 1. The client `POST`s the file bytes to the upload endpoint.
//! 2. The server stores the file (backed by RustFS/S3-compatible storage) and
//!    returns an `mxc://` URI.
//! 3. Other clients download or thumbnail the media by referencing that URI
//!    through the download/thumbnail endpoints.
//!
//! **Thumbnails** are generated on the fly (or served from cache) based on the
//! requested `width`, `height`, and `method` (`crop` or `scale`).
//!
//! **URL previews** allow clients to request an OpenGraph-style preview for an
//! arbitrary HTTP(S) URL, which the server fetches and summarises.
//!
//! Both the authenticated `v1` (Matrix v1.11+) paths and the legacy `v3` media
//! paths are supported for backwards compatibility.
//!
//! # Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | `POST` | `/_matrix/client/v1/media/upload` | Upload media (authenticated, v1.11+) |
//! | `GET`  | `/_matrix/client/v1/media/download/{serverName}/{mediaId}` | Download media |
//! | `GET`  | `/_matrix/client/v1/media/download/{serverName}/{mediaId}/{fileName}` | Download media with a suggested filename |
//! | `GET`  | `/_matrix/client/v1/media/thumbnail/{serverName}/{mediaId}` | Get a thumbnail of the media |
//! | `GET`  | `/_matrix/client/v1/media/config` | Get server media config (max upload size) |
//! | `GET`  | `/_matrix/client/v1/media/preview_url` | Get an OpenGraph preview for a URL |
//! | `POST` | `/_matrix/media/v3/upload` | Upload media (legacy v3 path) |
//! | `GET`  | `/_matrix/media/v3/download/{serverName}/{mediaId}` | Download media (legacy v3) |
//! | `GET`  | `/_matrix/media/v3/download/{serverName}/{mediaId}/{fileName}` | Download with filename (legacy v3) |
//! | `GET`  | `/_matrix/media/v3/thumbnail/{serverName}/{mediaId}` | Thumbnail (legacy v3) |
//! | `GET`  | `/_matrix/media/v3/config` | Media config (legacy v3) |
//! | `GET`  | `/_matrix/media/v3/preview_url` | URL preview (legacy v3) |
//!
//! # Matrix spec
//!
//! * [Content repository](https://spec.matrix.org/v1.12/client-server-api/#content-repository)

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tracing::debug;

use maelstrom_core::matrix::error::MatrixError;
use maelstrom_storage::traits::MediaRecord;

use crate::extractors::AuthenticatedUser;
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        // Authenticated endpoints (Matrix v1.11+)
        .route("/_matrix/client/v1/media/upload", post(upload))
        .route(
            "/_matrix/client/v1/media/download/{serverName}/{mediaId}",
            get(download),
        )
        .route(
            "/_matrix/client/v1/media/download/{serverName}/{mediaId}/{fileName}",
            get(download_with_filename),
        )
        .route(
            "/_matrix/client/v1/media/thumbnail/{serverName}/{mediaId}",
            get(thumbnail),
        )
        .route("/_matrix/client/v1/media/config", get(config))
        .route("/_matrix/client/v1/media/preview_url", get(preview_url))
        // Legacy v3 endpoints (backwards compat)
        .route("/_matrix/media/v3/upload", post(upload))
        .route(
            "/_matrix/media/v3/download/{serverName}/{mediaId}",
            get(download),
        )
        .route(
            "/_matrix/media/v3/download/{serverName}/{mediaId}/{fileName}",
            get(download_with_filename),
        )
        .route(
            "/_matrix/media/v3/thumbnail/{serverName}/{mediaId}",
            get(thumbnail),
        )
        .route("/_matrix/media/v3/config", get(config))
        .route("/_matrix/media/v3/preview_url", get(preview_url))
}

// -- Upload --

#[derive(Deserialize)]
struct UploadQuery {
    filename: Option<String>,
}

#[derive(Serialize)]
struct UploadResponse {
    content_uri: String,
}

async fn upload(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<UploadQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Result<Json<UploadResponse>, MatrixError> {
    let media_client = state
        .media()
        .ok_or_else(|| MatrixError::unknown("Media storage not configured"))?;

    // Validate size
    if body.len() as u64 > state.max_upload_size() {
        return Err(MatrixError::too_large(format!(
            "Upload exceeds maximum size of {} bytes",
            state.max_upload_size()
        )));
    }

    // Extract content type from header
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    // Generate media ID
    let media_id = uuid::Uuid::new_v4().to_string();
    let server_name = state.server_name().as_str().to_string();
    let s3_key = format!("{server_name}/{media_id}");

    debug!(
        media_id = %media_id,
        content_type = %content_type,
        size = body.len(),
        "Processing media upload"
    );

    // Upload to S3
    media_client
        .upload(&s3_key, body.clone(), &content_type)
        .await
        .map_err(crate::extractors::media_error)?;

    // Store metadata in DB
    let record = MediaRecord {
        media_id: media_id.clone(),
        server_name: server_name.clone(),
        user_id: auth.user_id.to_string(),
        content_type,
        content_length: body.len() as u64,
        filename: query.filename,
        s3_key,
        created_at: chrono::Utc::now(),
        quarantined: false,
    };

    state
        .storage()
        .store_media(&record)
        .await
        .map_err(crate::extractors::storage_error)?;

    let content_uri = format!("mxc://{server_name}/{media_id}");

    Ok(Json(UploadResponse { content_uri }))
}

// -- Download --

#[derive(Deserialize)]
struct DownloadParams {
    #[serde(rename = "serverName")]
    server_name: String,
    #[serde(rename = "mediaId")]
    media_id: String,
}

#[derive(Deserialize)]
struct DownloadWithFilenameParams {
    #[serde(rename = "serverName")]
    server_name: String,
    #[serde(rename = "mediaId")]
    media_id: String,
    #[serde(rename = "fileName")]
    file_name: String,
}

async fn download(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<DownloadParams>,
) -> Result<Response, MatrixError> {
    serve_media(&state, &params.server_name, &params.media_id, None).await
}

async fn download_with_filename(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<DownloadWithFilenameParams>,
) -> Result<Response, MatrixError> {
    serve_media(
        &state,
        &params.server_name,
        &params.media_id,
        Some(&params.file_name),
    )
    .await
}

async fn serve_media(
    state: &AppState,
    server_name: &str,
    media_id: &str,
    filename_override: Option<&str>,
) -> Result<Response, MatrixError> {
    // Try local storage first
    match state.storage().get_media(server_name, media_id).await {
        Ok(record) => {
            if record.quarantined {
                return Err(MatrixError::not_found("Media not found"));
            }

            let media_client = state
                .media()
                .ok_or_else(|| MatrixError::unknown("Media storage not configured"))?;

            let result = media_client
                .download(&record.s3_key)
                .await
                .map_err(crate::extractors::media_error)?;

            let filename = filename_override.map(|s| s.to_string()).or(record.filename);

            let mut builder = Response::builder()
                .header(header::CONTENT_TYPE, &record.content_type)
                .header(header::CONTENT_LENGTH, result.data.len());

            if let Some(ref name) = filename {
                builder = builder.header(
                    header::CONTENT_DISPOSITION,
                    format!("inline; filename=\"{name}\""),
                );
            }

            builder
                .body(Body::from(result.data))
                .map_err(|_| MatrixError::unknown("Failed to build response"))
        }
        Err(_) if server_name != state.server_name().as_str() => {
            // Remote media — proxy from the origin server
            proxy_remote_media(server_name, media_id, filename_override).await
        }
        Err(e) => Err(crate::extractors::storage_error(e)),
    }
}

/// Proxy a media download from a remote Matrix server.
async fn proxy_remote_media(
    server_name: &str,
    media_id: &str,
    filename_override: Option<&str>,
) -> Result<Response, MatrixError> {
    debug!(server_name = %server_name, media_id = %media_id, "Proxying remote media");

    let url = format!("https://{server_name}/_matrix/media/v3/download/{server_name}/{media_id}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| MatrixError::unknown("Failed to build HTTP client"))?;

    let resp = client.get(&url).send().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to fetch remote media");
        MatrixError::not_found("Remote media not available")
    })?;

    if !resp.status().is_success() {
        return Err(MatrixError::not_found("Remote media not found"));
    }

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let data = resp
        .bytes()
        .await
        .map_err(|_| MatrixError::unknown("Failed to read remote media"))?;

    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, &content_type)
        .header(header::CONTENT_LENGTH, data.len());

    if let Some(name) = filename_override {
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{name}\""),
        );
    }

    builder
        .body(Body::from(data))
        .map_err(|_| MatrixError::unknown("Failed to build response"))
}

// -- Thumbnail --

#[derive(Deserialize)]
struct ThumbnailQuery {
    width: Option<u32>,
    height: Option<u32>,
    method: Option<String>,
}

async fn thumbnail(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
    Path(params): Path<DownloadParams>,
    Query(query): Query<ThumbnailQuery>,
) -> Result<Response, MatrixError> {
    let media_client = state
        .media()
        .ok_or_else(|| MatrixError::unknown("Media storage not configured"))?;

    let record = state
        .storage()
        .get_media(&params.server_name, &params.media_id)
        .await
        .map_err(crate::extractors::storage_error)?;

    if record.quarantined {
        return Err(MatrixError::not_found("Media not found"));
    }

    let result = media_client
        .download(&record.s3_key)
        .await
        .map_err(crate::extractors::media_error)?;

    let width = query.width.unwrap_or(320);
    let height = query.height.unwrap_or(240);
    let method = query
        .method
        .as_deref()
        .map(maelstrom_media::thumbnail::ResizeMethod::parse)
        .unwrap_or(maelstrom_media::thumbnail::ResizeMethod::Scale);

    // Try to generate a thumbnail; fall back to original if not an image
    match maelstrom_media::thumbnail::generate(&result.data, width, height, method) {
        Ok(Some(thumb)) => Response::builder()
            .header(header::CONTENT_TYPE, &thumb.content_type)
            .header(header::CONTENT_LENGTH, thumb.data.len())
            .body(Body::from(thumb.data))
            .map_err(|_| MatrixError::unknown("Failed to build response")),
        Ok(None) | Err(_) => {
            // Not an image or resize failed — serve original per spec
            Response::builder()
                .header(header::CONTENT_TYPE, &record.content_type)
                .header(header::CONTENT_LENGTH, result.data.len())
                .body(Body::from(result.data))
                .map_err(|_| MatrixError::unknown("Failed to build response"))
        }
    }
}

// -- Config --

#[derive(Serialize)]
struct MediaConfigResponse {
    #[serde(rename = "m.upload.size")]
    upload_size: u64,
}

async fn config(
    State(state): State<AppState>,
    _auth: AuthenticatedUser,
) -> Result<Json<MediaConfigResponse>, MatrixError> {
    Ok(Json(MediaConfigResponse {
        upload_size: state.max_upload_size(),
    }))
}

// -- Preview URL --

#[derive(Deserialize)]
struct PreviewUrlQuery {
    url: String,
    #[allow(dead_code)]
    ts: Option<u64>,
}

async fn preview_url(
    State(_state): State<AppState>,
    _auth: AuthenticatedUser,
    Query(query): Query<PreviewUrlQuery>,
) -> Result<Json<maelstrom_media::preview::OgMetadata>, MatrixError> {
    let metadata = maelstrom_media::preview::fetch_og_metadata(&query.url)
        .await
        .map_err(crate::extractors::media_error)?;

    Ok(Json(metadata))
}
