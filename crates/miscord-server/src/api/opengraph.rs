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
}

#[derive(Debug, Deserialize)]
pub struct FetchOpenGraphQuery {
    pub url: String,
}

// Regex patterns for OpenGraph meta tags
static RE_OG_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+(?:property|name)=["']og:title["']\s+content=["']([^"']+)["']"#).unwrap()
});
static RE_OG_TITLE_ALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+content=["']([^"']+)["']\s+(?:property|name)=["']og:title["']"#).unwrap()
});
static RE_OG_DESC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+(?:property|name)=["']og:description["']\s+content=["']([^"']+)["']"#)
        .unwrap()
});
static RE_OG_DESC_ALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+content=["']([^"']+)["']\s+(?:property|name)=["']og:description["']"#)
        .unwrap()
});
static RE_OG_IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+(?:property|name)=["']og:image["']\s+content=["']([^"']+)["']"#).unwrap()
});
static RE_OG_IMAGE_ALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+content=["']([^"']+)["']\s+(?:property|name)=["']og:image["']"#).unwrap()
});
static RE_OG_SITE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+(?:property|name)=["']og:site_name["']\s+content=["']([^"']+)["']"#)
        .unwrap()
});
static RE_OG_SITE_NAME_ALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+content=["']([^"']+)["']\s+(?:property|name)=["']og:site_name["']"#)
        .unwrap()
});

// Fallback patterns for regular HTML meta tags
static RE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<title[^>]*>([^<]+)</title>"#).unwrap());
static RE_META_DESC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+name=["']description["']\s+content=["']([^"']+)["']"#).unwrap()
});
static RE_META_DESC_ALT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta\s+content=["']([^"']+)["']\s+name=["']description["']"#).unwrap()
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

    // Extract OpenGraph metadata
    let title = extract_match(&RE_OG_TITLE, body)
        .or_else(|| extract_match(&RE_OG_TITLE_ALT, body))
        .or_else(|| extract_match(&RE_TITLE, body));

    let description = extract_match(&RE_OG_DESC, body)
        .or_else(|| extract_match(&RE_OG_DESC_ALT, body))
        .or_else(|| extract_match(&RE_META_DESC, body))
        .or_else(|| extract_match(&RE_META_DESC_ALT, body));

    let image = extract_match(&RE_OG_IMAGE, body).or_else(|| extract_match(&RE_OG_IMAGE_ALT, body));

    let site_name = extract_match(&RE_OG_SITE_NAME, body)
        .or_else(|| extract_match(&RE_OG_SITE_NAME_ALT, body));

    Ok(Json(OpenGraphData {
        url: url.clone(),
        title: title.map(decode_html_entities),
        description: description.map(decode_html_entities),
        image,
        site_name: site_name.map(decode_html_entities),
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
