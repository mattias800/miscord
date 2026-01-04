use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};

pub async fn get<T: DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T> {
    let client = reqwest::Client::new();
    let mut request = client.get(url);

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Request failed with status {}: {}", status, text);
    }

    Ok(response.json().await?)
}

pub async fn post<T: DeserializeOwned, B: Serialize>(
    url: &str,
    body: &B,
    token: Option<&str>,
) -> Result<T> {
    let client = reqwest::Client::new();
    let mut request = client.post(url).json(body);

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Request failed with status {}: {}", status, text);
    }

    Ok(response.json().await?)
}

pub async fn post_empty<T: DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T> {
    let client = reqwest::Client::new();
    let mut request = client.post(url);

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Request failed with status {}: {}", status, text);
    }

    Ok(response.json().await?)
}

pub async fn patch<T: DeserializeOwned, B: Serialize>(
    url: &str,
    body: &B,
    token: Option<&str>,
) -> Result<T> {
    let client = reqwest::Client::new();
    let mut request = client.patch(url).json(body);

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Request failed with status {}: {}", status, text);
    }

    Ok(response.json().await?)
}

pub async fn delete(url: &str, token: Option<&str>) -> Result<()> {
    let client = reqwest::Client::new();
    let mut request = client.delete(url);

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Request failed with status {}: {}", status, text);
    }

    Ok(())
}
