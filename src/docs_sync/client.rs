//! Feishu Docs API client with tenant_access_token caching.
//!
//! Shares the same token caching pattern as `LarkChannel` in `src/channels/lark.rs`.

use anyhow::{bail, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

const FEISHU_BASE_URL: &str = "https://open.feishu.cn/open-apis";
/// Refresh token this many seconds before announced expiry.
const TOKEN_REFRESH_SKEW: Duration = Duration::from_secs(120);
/// Fallback TTL when `expire` field is absent.
const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(7200);
/// Rate limit for batch_update_blocks: 3 requests per second.
const BATCH_UPDATE_MIN_INTERVAL: Duration = Duration::from_millis(334);

/// Retry constants following src/providers/reliable.rs pattern.
const RETRY_MAX_ATTEMPTS: u32 = 3;
const RETRY_BASE_BACKOFF: Duration = Duration::from_secs(1);
const RETRY_BACKOFF_CAP: Duration = Duration::from_secs(10);
const RETRY_AFTER_CAP: Duration = Duration::from_secs(30);

/// A single block update operation for the Feishu Docs API.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BlockUpdate {
    /// Block ID to update.
    pub block_id: String,
    /// Update payload (block content).
    pub update_text_elements: serde_json::Value,
}

#[derive(Debug, Clone)]
struct CachedToken {
    value: String,
    refresh_after: Instant,
}

/// Feishu Docs API client with tenant_access_token caching.
pub struct FeishuDocsClient {
    app_id: String,
    app_secret: String,
    http: reqwest::Client,
    token: Arc<RwLock<Option<CachedToken>>>,
    last_batch_update: Arc<tokio::sync::Mutex<Instant>>,
}

impl FeishuDocsClient {
    /// Create a new Feishu Docs API client.
    pub fn new(app_id: String, app_secret: String) -> Self {
        let http = crate::config::schema::build_runtime_proxy_client("channel.feishu");
        Self {
            app_id,
            app_secret,
            http,
            token: Arc::new(RwLock::new(None)),
            last_batch_update: Arc::new(tokio::sync::Mutex::new(
                Instant::now() - BATCH_UPDATE_MIN_INTERVAL,
            )),
        }
    }

    /// Get or refresh tenant access token (cached with proactive refresh).
    async fn get_token(&self) -> Result<String> {
        // Fast path: read lock
        {
            let cached = self.token.read().await;
            if let Some(ref t) = *cached {
                if Instant::now() < t.refresh_after {
                    return Ok(t.value.clone());
                }
            }
        }

        // Slow path: write lock with double-checked locking
        let mut cached = self.token.write().await;

        // Re-check: another caller may have refreshed while we waited for the write lock
        if let Some(ref t) = *cached {
            if Instant::now() < t.refresh_after {
                return Ok(t.value.clone());
            }
        }

        let url = format!("{FEISHU_BASE_URL}/auth/v3/tenant_access_token/internal");
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp = self.http.post(&url).json(&body).send().await?;
        let status = resp.status();
        let data: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            bail!("Feishu tenant_access_token request failed: status={status}, body={data}");
        }

        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = data
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            bail!("Feishu tenant_access_token failed: {msg}");
        }

        let token_value = data
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing tenant_access_token"))?
            .to_string();

        let ttl_secs = data
            .get("expire")
            .or_else(|| data.get("expires_in"))
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TOKEN_TTL.as_secs());
        let ttl = Duration::from_secs(ttl_secs.max(1));
        let refresh_in = ttl
            .checked_sub(TOKEN_REFRESH_SKEW)
            .unwrap_or(Duration::from_secs(1));

        *cached = Some(CachedToken {
            value: token_value.clone(),
            refresh_after: Instant::now() + refresh_in,
        });

        Ok(token_value)
    }

    /// Send an HTTP request with retry and exponential backoff.
    /// Retries on 429 (with Retry-After parsing), 5xx, and network errors.
    /// Non-retryable 4xx (except 429) are returned immediately.
    async fn send_with_retry<F>(&self, build_request: F) -> Result<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut backoff = RETRY_BASE_BACKOFF;
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=RETRY_MAX_ATTEMPTS {
            let resp = match build_request().send().await {
                Ok(resp) => resp,
                Err(e) => {
                    if attempt == RETRY_MAX_ATTEMPTS {
                        return Err(e.into());
                    }
                    tracing::warn!(attempt, "Feishu request failed (network): {e}, retrying");
                    last_err = Some(anyhow::Error::from(e));
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RETRY_BACKOFF_CAP);
                    continue;
                }
            };

            let status = resp.status();

            // Success — return immediately
            if status.is_success() {
                return Ok(resp);
            }

            // Non-retryable 4xx (except 429) — let caller handle
            if status.is_client_error() && status.as_u16() != 429 {
                return Ok(resp);
            }

            // Last attempt — return as-is
            if attempt == RETRY_MAX_ATTEMPTS {
                return Ok(resp);
            }

            // 429: parse Retry-After header; 5xx: use exponential backoff
            let wait = if status.as_u16() == 429 {
                resp.headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|secs| Duration::from_secs_f64(secs.max(0.0)).min(RETRY_AFTER_CAP))
                    .unwrap_or(backoff)
            } else {
                backoff
            };

            tracing::warn!(
                attempt,
                status = %status,
                wait_ms = wait.as_millis() as u64,
                "Feishu request failed, retrying"
            );
            tokio::time::sleep(wait).await;
            backoff = (backoff * 2).min(RETRY_BACKOFF_CAP);
        }

        match last_err {
            Some(e) => Err(e),
            None => bail!("Feishu request failed after retries"),
        }
    }

    /// GET /docx/v1/documents/{id}/raw_content — fetch raw document text.
    pub async fn get_raw_content(&self, document_id: &str) -> Result<String> {
        let token = self.get_token().await?;
        let url = format!("{FEISHU_BASE_URL}/docx/v1/documents/{document_id}/raw_content");
        let resp = self
            .send_with_retry(|| self.http.get(&url).bearer_auth(&token))
            .await?;
        let status = resp.status();
        let data: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            bail!("Feishu get_raw_content failed: status={status}, body={data}");
        }
        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = data
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            bail!("Feishu get_raw_content error: {msg}");
        }
        let content = data
            .pointer("/data/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(content)
    }
    /// PATCH /docx/v1/documents/{id}/blocks/batch_update — update document blocks.
    /// Respects 3/s rate limit.
    pub async fn batch_update_blocks(
        &self,
        document_id: &str,
        updates: &[BlockUpdate],
    ) -> Result<()> {
        if updates.is_empty() {
            return Ok(());
        }
        // Rate limit: wait if needed
        {
            let mut last = self.last_batch_update.lock().await;
            let elapsed = last.elapsed();
            if elapsed < BATCH_UPDATE_MIN_INTERVAL {
                tokio::time::sleep(BATCH_UPDATE_MIN_INTERVAL - elapsed).await;
            }
            *last = Instant::now();
        }
        let token = self.get_token().await?;
        let url = format!("{FEISHU_BASE_URL}/docx/v1/documents/{document_id}/blocks/batch_update");
        let body = serde_json::json!({ "requests": updates });
        let resp = self
            .send_with_retry(|| self.http.patch(&url).bearer_auth(&token).json(&body))
            .await?;
        let status = resp.status();
        let data: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            bail!("Feishu batch_update_blocks failed: status={status}, body={data}");
        }
        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = data
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            bail!("Feishu batch_update_blocks error: {msg}");
        }
        Ok(())
    }
    /// POST /docx/v1/documents — create a new document.
    pub async fn create_document(&self, title: &str) -> Result<String> {
        let token = self.get_token().await?;
        let url = format!("{FEISHU_BASE_URL}/docx/v1/documents");
        let body = serde_json::json!({ "title": title });
        let resp = self
            .send_with_retry(|| self.http.post(&url).bearer_auth(&token).json(&body))
            .await?;
        let status = resp.status();
        let data: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            bail!("Feishu create_document failed: status={status}, body={data}");
        }
        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = data
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown");
            bail!("Feishu create_document error: {msg}");
        }
        let doc_id = data
            .pointer("/data/document/document_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing document_id in create response"))?
            .to_string();
        Ok(doc_id)
    }
}
