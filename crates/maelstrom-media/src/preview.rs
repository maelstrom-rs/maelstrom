use serde::Serialize;
use tracing::{debug, warn};

use crate::client::MediaError;

/// OpenGraph metadata extracted from a URL.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OgMetadata {
    #[serde(rename = "og:title", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "og:description", skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "og:image", skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(rename = "og:url", skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(rename = "og:site_name", skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    #[serde(rename = "matrix:image:size", skip_serializing_if = "Option::is_none")]
    pub image_size: Option<u64>,
}

/// Fetch a URL and extract OpenGraph metadata from its HTML.
///
/// Returns empty metadata on non-HTML responses or fetch failures,
/// rather than propagating errors — the Matrix spec treats preview as best-effort.
pub async fn fetch_og_metadata(url: &str) -> Result<OgMetadata, MediaError> {
    debug!(url = %url, "Fetching URL preview");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Maelstrom Matrix Homeserver")
        .build()
        .map_err(|e| MediaError::Connection(format!("Failed to build HTTP client: {e}")))?;

    let response = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(url = %url, error = %e, "Failed to fetch URL for preview");
            return Ok(OgMetadata::default());
        }
    };

    // Only parse HTML content
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/html") {
        debug!(url = %url, content_type = %content_type, "Non-HTML response, skipping OG extraction");
        return Ok(OgMetadata::default());
    }

    // Limit response body to 1 MiB to prevent abuse
    let body = match response.text().await {
        Ok(text) => {
            if text.len() > 1_048_576 {
                text[..1_048_576].to_string()
            } else {
                text
            }
        }
        Err(e) => {
            warn!(url = %url, error = %e, "Failed to read response body");
            return Ok(OgMetadata::default());
        }
    };

    Ok(extract_og_from_html(&body))
}

/// Parse HTML and extract OpenGraph meta tags.
fn extract_og_from_html(html: &str) -> OgMetadata {
    let document = scraper::Html::parse_document(html);
    let meta_selector = scraper::Selector::parse("meta[property]").unwrap();
    let title_selector = scraper::Selector::parse("title").unwrap();

    let mut meta = OgMetadata::default();

    for element in document.select(&meta_selector) {
        let property = element.attr("property").unwrap_or("");
        let content = element.attr("content").unwrap_or("").to_string();

        if content.is_empty() {
            continue;
        }

        match property {
            "og:title" => meta.title = Some(content),
            "og:description" => meta.description = Some(content),
            "og:image" => meta.image = Some(content),
            "og:url" => meta.url = Some(content),
            "og:site_name" => meta.site_name = Some(content),
            _ => {}
        }
    }

    // Fallback: use <title> tag if no og:title
    if meta.title.is_none()
        && let Some(title_el) = document.select(&title_selector).next()
    {
        let text = title_el.text().collect::<String>();
        if !text.is_empty() {
            meta.title = Some(text);
        }
    }

    // Fallback: use meta description if no og:description
    if meta.description.is_none() {
        let desc_selector = scraper::Selector::parse("meta[name='description']").unwrap();
        if let Some(desc_el) = document.select(&desc_selector).next()
            && let Some(content) = desc_el.attr("content")
            && !content.is_empty()
        {
            meta.description = Some(content.to_string());
        }
    }

    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_og_from_html() {
        let html = r#"
            <html>
            <head>
                <meta property="og:title" content="Test Page">
                <meta property="og:description" content="A test description">
                <meta property="og:image" content="https://example.com/image.png">
                <meta property="og:url" content="https://example.com">
                <meta property="og:site_name" content="Example">
                <title>Fallback Title</title>
            </head>
            <body></body>
            </html>
        "#;

        let meta = extract_og_from_html(html);
        assert_eq!(meta.title.as_deref(), Some("Test Page"));
        assert_eq!(meta.description.as_deref(), Some("A test description"));
        assert_eq!(meta.image.as_deref(), Some("https://example.com/image.png"));
        assert_eq!(meta.url.as_deref(), Some("https://example.com"));
        assert_eq!(meta.site_name.as_deref(), Some("Example"));
    }

    #[test]
    fn test_fallback_to_title_tag() {
        let html = r#"
            <html>
            <head>
                <title>Fallback Title</title>
            </head>
            <body></body>
            </html>
        "#;

        let meta = extract_og_from_html(html);
        assert_eq!(meta.title.as_deref(), Some("Fallback Title"));
    }

    #[test]
    fn test_fallback_to_meta_description() {
        let html = r#"
            <html>
            <head>
                <meta name="description" content="Meta desc">
            </head>
            <body></body>
            </html>
        "#;

        let meta = extract_og_from_html(html);
        assert_eq!(meta.description.as_deref(), Some("Meta desc"));
    }

    #[test]
    fn test_empty_html() {
        let meta = extract_og_from_html("");
        assert!(meta.title.is_none());
        assert!(meta.description.is_none());
        assert!(meta.image.is_none());
    }
}
