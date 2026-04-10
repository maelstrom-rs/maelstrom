//! Image thumbnail generation.
//!
//! Generates thumbnails from uploaded media on the fly, as required by
//! `GET /_matrix/media/v3/thumbnail/{serverName}/{mediaId}`.
//!
//! ## Resize methods
//!
//! The Matrix spec defines two resize methods, both implemented here via the
//! `image` crate:
//!
//! - [`ResizeMethod::Scale`] -- Fit the image within the requested `width x height`
//!   while preserving the original aspect ratio. The output may be smaller than
//!   the requested dimensions on one axis.
//!
//! - [`ResizeMethod::Crop`] -- Scale the image so it completely fills the requested
//!   dimensions, then center-crop to the exact size. Useful for square avatars.
//!
//! ## Supported formats
//!
//! Input: PNG, JPEG, GIF, and WebP (detected via magic bytes, not file extension).
//! Unsupported formats (SVG, TIFF, etc.) return `None` so the caller can serve
//! the original file instead.
//!
//! Output is always PNG, returned as raw bytes in a [`ThumbnailResult`] alongside
//! the `image/png` content type.

use bytes::Bytes;
use image::ImageFormat;
use image::imageops::FilterType;
use std::io::Cursor;
use tracing::debug;

use crate::client::MediaError;

/// Supported resize methods per the Matrix spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeMethod {
    /// Scale to fit within the given dimensions, preserving aspect ratio.
    Scale,
    /// Scale and crop to fill the given dimensions exactly.
    Crop,
}

impl ResizeMethod {
    pub fn parse(s: &str) -> Self {
        match s {
            "crop" => Self::Crop,
            _ => Self::Scale,
        }
    }
}

/// Generate a thumbnail from image bytes.
///
/// Returns `None` if the input is not a supported image format.
/// Returns the thumbnail as PNG bytes along with the content type.
pub fn generate(
    data: &Bytes,
    width: u32,
    height: u32,
    method: ResizeMethod,
) -> Result<Option<ThumbnailResult>, MediaError> {
    // Try to detect the image format
    let format = match image::guess_format(data) {
        Ok(f) => f,
        Err(_) => return Ok(None), // Not a recognized image — skip
    };

    // Only process raster image formats we support
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Ok(None);
    }

    debug!(
        width = width,
        height = height,
        method = ?method,
        format = ?format,
        "Generating thumbnail"
    );

    // For animated formats (GIF, WebP), return the original rather than
    // flattening to a static frame. The `image` crate's resize only operates
    // on single frames, so resizing animated images would lose animation.
    if matches!(format, ImageFormat::Gif | ImageFormat::WebP) {
        // Serve original for animated formats — clients handle display sizing
        return Ok(Some(ThumbnailResult {
            data: data.clone(),
            content_type: match format {
                ImageFormat::Gif => "image/gif".to_string(),
                ImageFormat::WebP => "image/webp".to_string(),
                _ => unreachable!(),
            },
        }));
    }

    let img = image::load_from_memory(data)
        .map_err(|e| MediaError::Upload(format!("Failed to decode image: {e}")))?;

    let thumb = match method {
        ResizeMethod::Scale => img.resize(width, height, FilterType::Lanczos3),
        ResizeMethod::Crop => img.resize_to_fill(width, height, FilterType::Lanczos3),
    };

    // Preserve source format where possible, fall back to PNG
    let (out_format, content_type) = match format {
        ImageFormat::Jpeg => (ImageFormat::Jpeg, "image/jpeg"),
        _ => (ImageFormat::Png, "image/png"),
    };

    let mut buf = Cursor::new(Vec::new());
    thumb
        .write_to(&mut buf, out_format)
        .map_err(|e| MediaError::Upload(format!("Failed to encode thumbnail: {e}")))?;

    Ok(Some(ThumbnailResult {
        data: Bytes::from(buf.into_inner()),
        content_type: content_type.to_string(),
    }))
}

/// Result of thumbnail generation.
pub struct ThumbnailResult {
    pub data: Bytes,
    pub content_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_png(w: u32, h: u32) -> Bytes {
        let img = image::RgbaImage::new(w, h);
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png).unwrap();
        Bytes::from(buf.into_inner())
    }

    #[test]
    fn test_scale_thumbnail() {
        let png = make_test_png(200, 200);
        let result = generate(&png, 50, 50, ResizeMethod::Scale)
            .unwrap()
            .expect("should produce thumbnail");
        assert_eq!(result.content_type, "image/png");
        assert!(!result.data.is_empty());

        // Verify the output is smaller than original
        let decoded = image::load_from_memory(&result.data).unwrap();
        assert!(decoded.width() <= 50);
        assert!(decoded.height() <= 50);
    }

    #[test]
    fn test_crop_thumbnail() {
        let png = make_test_png(300, 200);
        let result = generate(&png, 100, 100, ResizeMethod::Crop)
            .unwrap()
            .expect("should produce thumbnail");

        let decoded = image::load_from_memory(&result.data).unwrap();
        assert_eq!(decoded.width(), 100);
        assert_eq!(decoded.height(), 100);
    }

    #[test]
    fn test_non_image_returns_none() {
        let data = Bytes::from("not an image at all");
        let result = generate(&data, 50, 50, ResizeMethod::Scale).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_resize_method_parse() {
        assert_eq!(ResizeMethod::parse("crop"), ResizeMethod::Crop);
        assert_eq!(ResizeMethod::parse("scale"), ResizeMethod::Scale);
        assert_eq!(ResizeMethod::parse("unknown"), ResizeMethod::Scale);
    }
}
