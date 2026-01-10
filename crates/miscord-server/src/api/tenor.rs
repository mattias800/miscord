use crate::auth::AuthUser;
use crate::error::{AppError, Result};
use crate::state::AppState;
use axum::{extract::Query, extract::State, Json};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const TENOR_API_BASE: &str = "https://tenor.googleapis.com/v2";

/// A single GIF result from Tenor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenorGif {
    pub id: String,
    pub title: String,
    pub media_formats: TenorMediaFormats,
    #[serde(default)]
    pub content_description: String,
}

/// Available media formats for a GIF
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenorMediaFormats {
    pub gif: Option<TenorMedia>,
    pub tinygif: Option<TenorMedia>,
    pub nanogif: Option<TenorMedia>,
    #[serde(rename = "gifpreview")]
    pub gif_preview: Option<TenorMedia>,
}

/// A single media format with URL and dimensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenorMedia {
    pub url: String,
    #[serde(default)]
    pub dims: Vec<u32>,
    #[serde(default)]
    pub size: u64,
}

/// Response from our GIF endpoints
#[derive(Debug, Serialize)]
pub struct GifSearchResponse {
    pub results: Vec<TenorGif>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// Query parameters for GIF search
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<u32>,
    pub pos: Option<String>,
}

/// Query parameters for trending GIFs
#[derive(Debug, Deserialize)]
pub struct TrendingQuery {
    pub limit: Option<u32>,
    pub pos: Option<String>,
}

/// Tenor API response structure
#[derive(Debug, Deserialize)]
struct TenorApiResponse {
    results: Vec<TenorGif>,
    next: Option<String>,
}

/// Search for GIFs by query
pub async fn search_gifs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<GifSearchResponse>> {
    let api_key = state.config.tenor_api_key.as_ref().ok_or_else(|| {
        AppError::BadRequest("GIF search is not configured".to_string())
    })?;

    let limit = query.limit.unwrap_or(20).min(50);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("MiscordBot/1.0")
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to create HTTP client: {}", e)))?;

    let mut url = format!(
        "{}/search?q={}&key={}&client_key=miscord&limit={}&media_filter=gif,tinygif,nanogif",
        TENOR_API_BASE,
        urlencoding::encode(&query.q),
        urlencoding::encode(api_key),
        limit
    );

    if let Some(pos) = &query.pos {
        url.push_str(&format!("&pos={}", urlencoding::encode(pos)));
    }

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to fetch from Tenor: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("Tenor API error: {} - {}", status, body);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Tenor API returned status {}",
            status
        )));
    }

    let tenor_response: TenorApiResponse = response
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to parse Tenor response: {}", e)))?;

    Ok(Json(GifSearchResponse {
        results: tenor_response.results,
        next: tenor_response.next,
    }))
}

/// Get trending/featured GIFs
pub async fn trending_gifs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(query): Query<TrendingQuery>,
) -> Result<Json<GifSearchResponse>> {
    let api_key = state.config.tenor_api_key.as_ref().ok_or_else(|| {
        AppError::BadRequest("GIF search is not configured".to_string())
    })?;

    let limit = query.limit.unwrap_or(20).min(50);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("MiscordBot/1.0")
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to create HTTP client: {}", e)))?;

    let mut url = format!(
        "{}/featured?key={}&client_key=miscord&limit={}&media_filter=gif,tinygif,nanogif",
        TENOR_API_BASE,
        urlencoding::encode(api_key),
        limit
    );

    if let Some(pos) = &query.pos {
        url.push_str(&format!("&pos={}", urlencoding::encode(pos)));
    }

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to fetch from Tenor: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("Tenor API error: {} - {}", status, body);
        return Err(AppError::Internal(anyhow::anyhow!(
            "Tenor API returned status {}",
            status
        )));
    }

    let tenor_response: TenorApiResponse = response
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to parse Tenor response: {}", e)))?;

    Ok(Json(GifSearchResponse {
        results: tenor_response.results,
        next: tenor_response.next,
    }))
}
