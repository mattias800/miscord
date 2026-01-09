use crate::auth::AuthUser;
use crate::error::{AppError, Result};
use axum::{extract::Query, Json};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;

/// OpenGraph metadata response
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenGraphData {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
    /// For video embeds: "youtube", "vimeo", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_type: Option<String>,
    /// Channel/author name for videos
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FetchOpenGraphQuery {
    pub url: String,
}

// Regex pattern to find meta tags (captures the entire tag content)
static RE_META_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<meta\s+([^>]+?)/?>"#).unwrap()
});

// Patterns to extract property/name and content from meta tag attributes
static RE_PROPERTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:property|name)\s*=\s*["']([^"']+)["']"#).unwrap()
});
static RE_CONTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)content\s*=\s*["']([^"']+)["']"#).unwrap()
});

// Fallback patterns for regular HTML meta tags
static RE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<title[^>]*>([^<]+)</title>"#).unwrap());

// YouTube URL patterns
static RE_YOUTUBE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:youtube\.com/watch\?v=|youtu\.be/|youtube\.com/embed/|youtube\.com/shorts/)([a-zA-Z0-9_-]+)"#).unwrap()
});

/// Fetch OpenGraph metadata for a URL
pub async fn fetch_opengraph(
    _auth: AuthUser,
    Query(query): Query<FetchOpenGraphQuery>,
) -> Result<Json<OpenGraphData>> {
    let url = &query.url;

    // Basic URL validation
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AppError::BadRequest("Invalid URL".to_string()));
    }

    // Check if this is a YouTube URL - use oEmbed API for better metadata
    if let Some(video_id) = extract_youtube_id(url) {
        return fetch_youtube_oembed(url, &video_id).await;
    }

    // Fetch the page with timeout
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("MiscordBot/1.0")
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to create HTTP client: {}", e)))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to fetch URL: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "URL returned status {}",
            response.status()
        )));
    }

    // Only process HTML content
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/html") {
        return Ok(Json(OpenGraphData {
            url: url.clone(),
            ..Default::default()
        }));
    }

    // Limit body size to 1MB
    let body = response
        .text()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to read response: {}", e)))?;

    let body = if body.len() > 1_000_000 {
        &body[..1_000_000]
    } else {
        &body
    };

    // Extract OpenGraph metadata by parsing all meta tags
    let mut og_title = None;
    let mut og_description = None;
    let mut og_image = None;
    let mut og_site_name = None;
    let mut meta_description = None;

    for meta_cap in RE_META_TAG.captures_iter(body) {
        let attrs = &meta_cap[1];

        // Extract property/name and content from this meta tag
        let property = RE_PROPERTY
            .captures(attrs)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_lowercase());
        let content = RE_CONTENT
            .captures(attrs)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        if let (Some(prop), Some(cont)) = (property, content) {
            match prop.as_str() {
                "og:title" => og_title = og_title.or(Some(cont)),
                "og:description" => og_description = og_description.or(Some(cont)),
                "og:image" => og_image = og_image.or(Some(cont)),
                "og:site_name" => og_site_name = og_site_name.or(Some(cont)),
                "description" => meta_description = meta_description.or(Some(cont)),
                _ => {}
            }
        }
    }

    // Fallback to <title> tag if no og:title
    let title = og_title.or_else(|| extract_match(&RE_TITLE, body));

    // Fallback to meta description if no og:description
    let description = og_description.or(meta_description);

    // Fallback to domain name if no og:site_name
    let site_name = og_site_name.or_else(|| extract_domain(url));

    Ok(Json(OpenGraphData {
        url: url.clone(),
        title: title.map(decode_html_entities),
        description: description.map(decode_html_entities),
        image: og_image.map(decode_html_entities),
        site_name: site_name.map(decode_html_entities),
        video_type: None,
        author_name: None,
    }))
}

fn extract_match(re: &Regex, text: &str) -> Option<String> {
    re.captures(text)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

fn decode_html_entities(s: String) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Extract domain from URL (e.g., "https://www.example.com/path" -> "www.example.com")
fn extract_domain(url: &str) -> Option<String> {
    let url = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let domain = url.split('/').next()?;
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

/// Extract YouTube video ID from various URL formats
fn extract_youtube_id(url: &str) -> Option<String> {
    RE_YOUTUBE
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// YouTube oEmbed response
#[derive(Debug, Deserialize)]
struct YouTubeOEmbed {
    title: Option<String>,
    author_name: Option<String>,
    thumbnail_url: Option<String>,
}

/// Fetch YouTube video metadata via oEmbed API
async fn fetch_youtube_oembed(original_url: &str, video_id: &str) -> Result<Json<OpenGraphData>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("MiscordBot/1.0")
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to create HTTP client: {}", e)))?;

    // Use canonical YouTube URL for oEmbed
    let video_url = format!("https://www.youtube.com/watch?v={}", video_id);
    let oembed_url = format!(
        "https://www.youtube.com/oembed?url={}&format=json",
        urlencoding::encode(&video_url)
    );

    let response = client
        .get(&oembed_url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to fetch YouTube oEmbed: {}", e)))?;

    if !response.status().is_success() {
        // Fall back to basic info if oEmbed fails
        return Ok(Json(OpenGraphData {
            url: original_url.to_string(),
            title: None,
            description: None,
            image: Some(format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id)),
            site_name: Some("YouTube".to_string()),
            video_type: Some("youtube".to_string()),
            author_name: None,
        }));
    }

    let oembed: YouTubeOEmbed = response
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to parse YouTube oEmbed: {}", e)))?;

    // Use high-quality thumbnail (maxresdefault if available, fallback to hqdefault)
    let thumbnail = oembed.thumbnail_url.unwrap_or_else(|| {
        format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id)
    });

    Ok(Json(OpenGraphData {
        url: original_url.to_string(),
        title: oembed.title,
        description: None, // YouTube oEmbed doesn't include description
        image: Some(thumbnail),
        site_name: Some("YouTube".to_string()),
        video_type: Some("youtube".to_string()),
        author_name: oembed.author_name,
    }))
}
