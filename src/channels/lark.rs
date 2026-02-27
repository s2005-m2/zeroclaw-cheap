use super::lark_ws_manager::LarkWsManager;
use super::traits::{Channel, ChannelMessage, SendMessage};
use crate::config::StreamMode;
use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use uuid::Uuid;

const FEISHU_BASE_URL: &str = "https://open.feishu.cn/open-apis";
const FEISHU_WS_BASE_URL: &str = "https://open.feishu.cn";
const LARK_BASE_URL: &str = "https://open.larksuite.com/open-apis";
const LARK_WS_BASE_URL: &str = "https://open.larksuite.com";
const ACK_REACTION_EMOJI_TYPE: &str = "MUSCLE";
const LARK_MAX_FILE_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const LARK_MAX_FILE_DOWNLOAD_BYTES: usize = 20 * 1024 * 1024;

/// Attachment types recognized in outgoing Lark messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LarkAttachmentKind {
    Image,
    Document,
    Audio,
    Video,
}


/// LarkEvent envelope (method=1 / type=event payload)
#[derive(Debug, serde::Deserialize)]
struct LarkEvent {
    header: LarkEventHeader,
    event: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
struct LarkEventHeader {
    event_type: String,
    #[allow(dead_code)]
    event_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct MsgReceivePayload {
    sender: LarkSender,
    message: LarkMessage,
}

#[derive(Debug, serde::Deserialize)]
struct LarkSender {
    sender_id: LarkSenderId,
    #[serde(default)]
    sender_type: String,
}

#[derive(Debug, serde::Deserialize, Default)]
struct LarkSenderId {
    open_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LarkMessage {
    message_id: String,
    chat_id: String,
    chat_type: String,
    message_type: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    mentions: Vec<serde_json::Value>,
}

/// Feishu/Lark API business code for expired/invalid tenant access token.
const LARK_INVALID_ACCESS_TOKEN_CODE: i64 = 99_991_663;
/// Refresh tenant token this many seconds before the announced expiry.
const LARK_TOKEN_REFRESH_SKEW: Duration = Duration::from_secs(120);
/// Fallback tenant token TTL when `expire`/`expires_in` is absent.
const LARK_DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(7200);

#[derive(Debug, Clone)]
struct CachedTenantToken {
    value: String,
    refresh_after: Instant,
}

fn extract_lark_response_code(body: &serde_json::Value) -> Option<i64> {
    body.get("code").and_then(|c| c.as_i64())
}

fn is_lark_invalid_access_token(body: &serde_json::Value) -> bool {
    extract_lark_response_code(body) == Some(LARK_INVALID_ACCESS_TOKEN_CODE)
}

fn should_refresh_lark_tenant_token(status: reqwest::StatusCode, body: &serde_json::Value) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || is_lark_invalid_access_token(body)
}

fn extract_lark_token_ttl_seconds(body: &serde_json::Value) -> u64 {
    let ttl = body
        .get("expire")
        .or_else(|| body.get("expires_in"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            body.get("expire")
                .or_else(|| body.get("expires_in"))
                .and_then(|v| v.as_i64())
                .and_then(|v| u64::try_from(v).ok())
        })
        .unwrap_or(LARK_DEFAULT_TOKEN_TTL.as_secs());
    ttl.max(1)
}

fn next_token_refresh_deadline(now: Instant, ttl_seconds: u64) -> Instant {
    let ttl = Duration::from_secs(ttl_seconds.max(1));
    let refresh_in = ttl
        .checked_sub(LARK_TOKEN_REFRESH_SKEW)
        .unwrap_or(Duration::from_secs(1));
    now + refresh_in
}

fn ensure_lark_send_success(
    status: reqwest::StatusCode,
    body: &serde_json::Value,
    context: &str,
) -> anyhow::Result<()> {
    if !status.is_success() {
        anyhow::bail!("Lark send failed {context}: status={status}, body={body}");
    }

    let code = extract_lark_response_code(body).unwrap_or(0);
    if code != 0 {
        anyhow::bail!("Lark send failed {context}: code={code}, body={body}");
    }

    Ok(())
}

/// Platform selection for Lark (international) vs Feishu (China).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LarkPlatform {
    Lark,
    Feishu,
}

impl LarkPlatform {
    fn api_base(self) -> &'static str {
        match self {
            Self::Lark => LARK_BASE_URL,
            Self::Feishu => FEISHU_BASE_URL,
        }
    }

    fn ws_base(self) -> &'static str {
        match self {
            Self::Lark => LARK_WS_BASE_URL,
            Self::Feishu => FEISHU_WS_BASE_URL,
        }
    }

    fn channel_name(self) -> &'static str {
        match self {
            Self::Lark => "lark",
            Self::Feishu => "feishu",
        }
    }

    fn proxy_service_key(self) -> &'static str {
        match self {
            Self::Lark => "lark",
            Self::Feishu => "feishu",
        }
    }

    fn locale_header(self) -> &'static str {
        match self {
            Self::Lark => "en_us",
            Self::Feishu => "zh_cn",
        }
    }
}

/// Lark/Feishu channel.
///
/// Supports two receive modes (configured via `receive_mode` in config):
/// - **`websocket`** (default): persistent WSS long-connection; no public URL needed.
/// - **`webhook`**: HTTP callback server; requires a public HTTPS endpoint.
#[derive(Clone)]
pub struct LarkChannel {
    app_id: String,
    app_secret: String,
    verification_token: String,
    port: Option<u16>,
    allowed_users: Vec<String>,
    /// Runtime endpoint/platform selection.
    platform: LarkPlatform,
    /// How to receive events: WebSocket long-connection or HTTP webhook.
    receive_mode: crate::config::schema::LarkReceiveMode,
    /// Cached tenant access token
    tenant_token: Arc<RwLock<Option<CachedTenantToken>>>,
    /// Dedup set: WS message_ids seen in last ~30 min to prevent double-dispatch
    ws_seen_ids: Arc<RwLock<HashMap<String, Instant>>>,
    /// Streaming mode for progressive draft updates via CardKit.
    stream_mode: StreamMode,
    /// Minimum interval (ms) between card updates.
    draft_update_interval_ms: u64,
    /// CardKit card sequence numbers per card_id.
    card_sequence: Arc<std::sync::Mutex<HashMap<String, u64>>>,
    /// Throttle tracking: last draft update time per card_id.
    last_draft_update: Arc<std::sync::Mutex<HashMap<String, Instant>>>,
    /// Typing indicator card IDs per recipient (for "正在处理..." cards).
    typing_card_ids: Arc<std::sync::Mutex<HashMap<String, String>>>,
    /// Optional docs_sync sharer for auto-sharing documents with new users.
    #[cfg(feature = "feishu-docs-sync")]
    docs_sharer: Option<std::sync::Arc<crate::docs_sync::DocsSyncSharer>>,
    /// Shared WS connection manager (None when using webhook mode).
    ws_manager: Option<Arc<LarkWsManager>>,
}

impl LarkChannel {
    pub fn new(
        app_id: String,
        app_secret: String,
        verification_token: String,
        port: Option<u16>,
        allowed_users: Vec<String>,
    ) -> Self {
        Self::new_with_platform(
            app_id,
            app_secret,
            verification_token,
            port,
            allowed_users,
            LarkPlatform::Lark,
        )
    }

    fn new_with_platform(
        app_id: String,
        app_secret: String,
        verification_token: String,
        port: Option<u16>,
        allowed_users: Vec<String>,
        platform: LarkPlatform,
    ) -> Self {
        Self {
            app_id,
            app_secret,
            verification_token,
            port,
            allowed_users,
            platform,
            receive_mode: crate::config::schema::LarkReceiveMode::default(),
            tenant_token: Arc::new(RwLock::new(None)),
            ws_seen_ids: Arc::new(RwLock::new(HashMap::new())),
            stream_mode: StreamMode::default(),
            draft_update_interval_ms: 500,
            card_sequence: Arc::new(std::sync::Mutex::new(HashMap::new())),
            last_draft_update: Arc::new(std::sync::Mutex::new(HashMap::new())),
            typing_card_ids: Arc::new(std::sync::Mutex::new(HashMap::new())),
            ws_manager: None,
            #[cfg(feature = "feishu-docs-sync")]
            docs_sharer: None,
        }
    }

    /// Build from `LarkConfig` using legacy compatibility:
    /// when `use_feishu=true`, this instance routes to Feishu endpoints.
    pub fn from_config(config: &crate::config::schema::LarkConfig) -> Self {
        let platform = if config.use_feishu {
            LarkPlatform::Feishu
        } else {
            LarkPlatform::Lark
        };
        let mut ch = Self::new_with_platform(
            config.app_id.clone(),
            config.app_secret.clone(),
            config.verification_token.clone().unwrap_or_default(),
            config.port,
            config.allowed_users.clone(),
            platform,
        );
        ch.receive_mode = config.receive_mode.clone();
        ch.stream_mode = StreamMode::Off; // CardKit update_card API is broken ("card is required"); disable streaming
        ch.draft_update_interval_ms = config.draft_update_interval_ms;
        ch
    }

    /// Build from explicit Lark config entry.
    pub fn from_lark_config(config: &crate::config::schema::LarkConfig) -> Self {
        let mut ch = Self::new_with_platform(
            config.app_id.clone(),
            config.app_secret.clone(),
            config.verification_token.clone().unwrap_or_default(),
            config.port,
            config.allowed_users.clone(),
            LarkPlatform::Lark,
        );
        ch.receive_mode = config.receive_mode.clone();
        ch.stream_mode = StreamMode::Off; // CardKit update_card API is broken ("card is required"); disable streaming
        ch.draft_update_interval_ms = config.draft_update_interval_ms;
        ch
    }

    /// Build from explicit Feishu config entry.
    pub fn from_feishu_config(config: &crate::config::schema::FeishuConfig) -> Self {
        let mut ch = Self::new_with_platform(
            config.app_id.clone(),
            config.app_secret.clone(),
            config.verification_token.clone().unwrap_or_default(),
            config.port,
            config.allowed_users.clone(),
            LarkPlatform::Feishu,
        );
        ch.receive_mode = config.receive_mode.clone();
        ch.stream_mode = StreamMode::Off; // CardKit update_card API is broken ("card is required"); disable streaming
        ch.draft_update_interval_ms = config.draft_update_interval_ms;
        ch
    }

    /// Configure streaming mode for progressive draft updates via CardKit.
    pub fn with_streaming(mut self, stream_mode: StreamMode, draft_update_interval_ms: u64) -> Self {
        self.stream_mode = stream_mode;
        self.draft_update_interval_ms = draft_update_interval_ms;
        self
    }

    /// Set the docs_sync sharer for auto-sharing documents with Feishu users.
    #[cfg(feature = "feishu-docs-sync")]
    pub fn set_docs_sharer(&mut self, sharer: std::sync::Arc<crate::docs_sync::DocsSyncSharer>) {
        self.docs_sharer = Some(sharer);
    }

    /// Set the shared WS connection manager.
    pub fn set_ws_manager(&mut self, manager: Arc<LarkWsManager>) {
        self.ws_manager = Some(manager);
    }

    fn http_client(&self) -> reqwest::Client {
        crate::config::build_runtime_proxy_client(self.platform.proxy_service_key())
    }

    fn channel_name(&self) -> &'static str {
        self.platform.channel_name()
    }

    fn api_base(&self) -> &'static str {
        self.platform.api_base()
    }

    fn ws_base(&self) -> &'static str {
        self.platform.ws_base()
    }

    fn tenant_access_token_url(&self) -> String {
        format!("{}/auth/v3/tenant_access_token/internal", self.api_base())
    }

    fn send_message_url(&self) -> String {
        format!("{}/im/v1/messages?receive_id_type=chat_id", self.api_base())
    }

    fn upload_image_url(&self) -> String {
        format!("{}/im/v1/images", self.api_base())
    }

    fn upload_file_url(&self) -> String {
        format!("{}/im/v1/files", self.api_base())
    }

    fn cardkit_url(&self) -> String {
        format!("{}/cardkit/v1/cards", self.api_base())
    }


    fn download_image_url(&self, message_id: &str, image_key: &str) -> String {
        format!(
            "{}/im/v1/messages/{message_id}/resources/{image_key}?type=image",
            self.api_base()
        )
    }

    /// URL for downloading file/audio/media resources from a message.
    fn download_file_url(&self, message_id: &str, file_key: &str, resource_type: &str) -> String {
        format!(
            "{}/im/v1/messages/{message_id}/resources/{file_key}?type={resource_type}",
            self.api_base()
        )
    }

    fn message_reaction_url(&self, message_id: &str) -> String {
        format!("{}/im/v1/messages/{message_id}/reactions", self.api_base())
    }

    async fn post_message_reaction_with_token(
        &self,
        message_id: &str,
        token: &str,
        emoji_type: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let url = self.message_reaction_url(message_id);
        let body = serde_json::json!({
            "reaction_type": {
                "emoji_type": emoji_type
            }
        });

        let response = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await?;

        Ok(response)
    }

    /// Best-effort "received" signal for incoming messages.
    /// Failures are logged and never block normal message handling.
    async fn try_add_ack_reaction(&self, message_id: &str, emoji_type: &str) {
        if message_id.is_empty() {
            return;
        }

        let mut token = match self.get_tenant_access_token().await {
            Ok(token) => token,
            Err(err) => {
                tracing::warn!("Lark: failed to fetch token for reaction: {err}");
                return;
            }
        };

        let mut retried = false;
        loop {
            let response = match self
                .post_message_reaction_with_token(message_id, &token, emoji_type)
                .await
            {
                Ok(resp) => resp,
                Err(err) => {
                    tracing::warn!("Lark: failed to add reaction for {message_id}: {err}");
                    return;
                }
            };

            let resp_status = response.status();
            // Parse body early so we can check both HTTP 401 and Lark business code
            let resp_body: serde_json::Value = match response.json().await {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!("Lark: add reaction decode failed for {message_id}: {err}");
                    return;
                }
            };

            if !retried && should_refresh_lark_tenant_token(resp_status, &resp_body) {
                self.invalidate_token().await;
                token = match self.get_tenant_access_token().await {
                    Ok(new_token) => new_token,
                    Err(err) => {
                        tracing::warn!(
                            "Lark: failed to refresh token for reaction on {message_id}: {err}"
                        );
                        return;
                    }
                };
                retried = true;
                continue;
            }

            if !resp_status.is_success() {
                tracing::warn!(
                    "Lark: add reaction failed for {message_id}: status={resp_status}, body={resp_body}"
                );
                return;
            }
            let code = resp_body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
            if code != 0 {
                let msg = resp_body
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                tracing::warn!("Lark: add reaction returned code={code} for {message_id}: {msg}");
            }
            return;
        }
    }

    /// WS event loop — receives decoded events from the shared `LarkWsManager`.
    #[allow(clippy::too_many_lines)]
    async fn listen_ws(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let manager = self.ws_manager.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Lark: ws_manager not set"))?;
        let mut rx = manager.subscribe();

        // Overflow buffer for messages that couldn't be sent (channel full).
        const OVERFLOW_CAP: usize = 20;
        let mut overflow: VecDeque<ChannelMessage> = VecDeque::new();

        loop {
            // Drain overflow buffer first (non-blocking)
            while let Some(msg) = overflow.front() {
                match tx.try_send(msg.clone()) {
                    Ok(()) => { overflow.pop_front(); }
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => break,
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => return Ok(()),
                }
            }

            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Lark: broadcast lagged, skipped {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    anyhow::bail!("Lark: WS manager broadcast closed");
                }
            };

            if event.event_type != "im.message.receive_v1" { continue; }

            let event_value: LarkEvent = match serde_json::from_slice(&event.payload) {
                Ok(e) => e,
                Err(e) => { tracing::error!("Lark: event JSON: {e}"); continue; }
            };

            let event_payload = event_value.event;

            let recv: MsgReceivePayload = match serde_json::from_value(event_payload.clone()) {
                Ok(r) => r,
                Err(e) => { tracing::error!("Lark: payload parse: {e}"); continue; }
            };

            if recv.sender.sender_type == "app" || recv.sender.sender_type == "bot" { continue; }

            let sender_open_id = recv.sender.sender_id.open_id.as_deref().unwrap_or("");
            if !self.is_user_allowed(sender_open_id) {
                tracing::warn!("Lark WS: ignoring {sender_open_id} (not in allowed_users)");
                continue;
            }

            // Auto-share docs_sync documents with new users
            #[cfg(feature = "feishu-docs-sync")]
            if let Some(ref sharer) = self.docs_sharer {
                let sharer = std::sync::Arc::clone(sharer);
                let oid = sender_open_id.to_string();
                tokio::spawn(async move {
                    if let Err(e) = sharer.share_all_docs_with(&oid).await {
                        tracing::warn!("docs_sync: auto-share failed for {oid}: {e}");
                    }
                });
            }

            let lark_msg = &recv.message;

            // Dedup
            {
                let now = Instant::now();
                let mut seen = self.ws_seen_ids.write().await;
                // GC
                seen.retain(|_, t| now.duration_since(*t) < Duration::from_secs(30 * 60));
                if seen.contains_key(&lark_msg.message_id) {
                    tracing::debug!("Lark WS: dup {}", lark_msg.message_id);
                    continue;
                }
                seen.insert(lark_msg.message_id.clone(), now);
            }

            // Decode content by type (mirrors clawdbot-feishu parsing)
            let text = match lark_msg.message_type.as_str() {
                "text" => {
                    let v: serde_json::Value = match serde_json::from_str(&lark_msg.content) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    match v.get("text").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
                        Some(t) => t.to_string(),
                        None => continue,
                    }
                }
                "post" => match parse_post_content(&lark_msg.content) {
                    Some(t) => t,
                    None => continue,
                },
                "image" => {
                    match extract_image_key(&lark_msg.content) {
                        Some(key) => format!("[IMAGE:lark_image_key:{key}]"),
                        None => continue,
                    }
                }
                "file" => match extract_file_key_and_name(&lark_msg.content) {
                    Some((key, name)) => {
                        let label = name.as_deref().unwrap_or(&key);
                        format!("[DOCUMENT:lark_file_key:{key}:{label}]")
                    }
                    None => continue,
                },
                "audio" => match extract_file_key(&lark_msg.content) {
                    Some(key) => format!("[AUDIO:lark_file_key:{key}]"),
                    None => continue,
                },
                "media" => match extract_file_key_and_name(&lark_msg.content) {
                    Some((key, name)) => {
                        let label = name.as_deref().unwrap_or(&key);
                        format!("[VIDEO:lark_file_key:{key}:{label}]")
                    }
                    None => continue,
                },
                _ => continue,
            };

            // Strip @_user_N placeholders
            let text = strip_at_placeholders(&text);
            let text = text.trim().to_string();
            if text.is_empty() { continue; }

            // Group-chat: only respond when explicitly @-mentioned
            if lark_msg.chat_type == "group" && !should_respond_in_group(&lark_msg.mentions) {
                continue;
            }

            let ack_emoji = "OK";
            let reaction_channel = self.clone();
            let reaction_message_id = lark_msg.message_id.clone();
            tokio::spawn(async move {
                reaction_channel
                    .try_add_ack_reaction(&reaction_message_id, &ack_emoji)
                    .await;
            });

            let channel_msg = ChannelMessage {
                id: Uuid::new_v4().to_string(),
                sender: lark_msg.chat_id.clone(),
                reply_target: lark_msg.chat_id.clone(),
                content: text,
                channel: self.channel_name().to_string(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                thread_ts: None,
            };

            tracing::debug!("Lark WS: message in {}", lark_msg.chat_id);
            match tx.try_send(channel_msg) {
                Ok(()) => {}
                Err(tokio::sync::mpsc::error::TrySendError::Full(msg)) => {
                    if overflow.len() >= OVERFLOW_CAP {
                        tracing::warn!("Lark WS: overflow buffer full, dropping oldest message");
                        overflow.pop_front();
                    }
                    tracing::warn!("Lark WS: channel full, buffering message for {}", msg.sender);
                    overflow.push_back(msg);
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
        Ok(())
    }

    /// Check if a user open_id is allowed
    fn is_user_allowed(&self, open_id: &str) -> bool {
        self.allowed_users.iter().any(|u| u == "*" || u == open_id)
    }

    /// Get or refresh tenant access token
    async fn get_tenant_access_token(&self) -> anyhow::Result<String> {
        // Check cache first
        {
            let cached = self.tenant_token.read().await;
            if let Some(ref token) = *cached {
                if Instant::now() < token.refresh_after {
                    return Ok(token.value.clone());
                }
            }
        }

        let url = self.tenant_access_token_url();
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp = self.http_client().post(&url).json(&body).send().await?;
        let status = resp.status();
        let data: serde_json::Value = resp.json().await?;

        if !status.is_success() {
            anyhow::bail!("Lark tenant_access_token request failed: status={status}, body={data}");
        }

        let code = data.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = data
                .get("msg")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("Lark tenant_access_token failed: {msg}");
        }

        let token = data
            .get("tenant_access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing tenant_access_token in response"))?
            .to_string();

        let ttl_seconds = extract_lark_token_ttl_seconds(&data);
        let refresh_after = next_token_refresh_deadline(Instant::now(), ttl_seconds);

        // Cache it with proactive refresh metadata.
        {
            let mut cached = self.tenant_token.write().await;
            *cached = Some(CachedTenantToken {
                value: token.clone(),
                refresh_after,
            });
        }

        Ok(token)
    }

    /// Invalidate cached token (called when API reports an expired tenant token).
    async fn invalidate_token(&self) {
        let mut cached = self.tenant_token.write().await;
        *cached = None;
    }

    /// Upload an image to Lark and return the `image_key`.
    async fn upload_image(&self, image_bytes: Vec<u8>, filename: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let url = self.upload_image_url();
        let part = reqwest::multipart::Part::bytes(image_bytes)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")?;
        let form = reqwest::multipart::Form::new()
            .text("image_type", "message")
            .part("image", part);
        let resp = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(form)
            .send()
            .await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("Lark upload_image failed: status={status}, body={body}");
        }
        let code = extract_lark_response_code(&body).unwrap_or(-1);
        if code != 0 {
            anyhow::bail!("Lark upload_image failed: code={code}, body={body}");
        }
        body.pointer("/data/image_key")
            .and_then(|k| k.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Lark upload_image: missing image_key in response"))
    }
    /// Download an image from a received message.
    async fn download_image(&self, message_id: &str, image_key: &str) -> anyhow::Result<Vec<u8>> {
        let token = self.get_tenant_access_token().await?;
        let url = self.download_image_url(message_id, image_key);
        let resp = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            anyhow::bail!("Lark download_image failed: status={status}");
        }
        Ok(resp.bytes().await?.to_vec())
    }
    /// Download a file/audio/media resource from a received message.
    ///
    /// `resource_type` should be `"image"` for images, `"file"` for file/audio/video.
    /// Rejects responses exceeding `LARK_MAX_FILE_DOWNLOAD_BYTES`.
    async fn download_file(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let token = self.get_tenant_access_token().await?;
        let url = self.download_file_url(message_id, file_key, resource_type);
        let resp = self
            .http_client()
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            anyhow::bail!("Lark download_file failed: status={status}");
        }
        let bytes = resp.bytes().await?.to_vec();
        if bytes.len() > LARK_MAX_FILE_DOWNLOAD_BYTES {
            anyhow::bail!(
                "Lark: downloaded file exceeds max size ({} bytes > {} MB)",
                bytes.len(),
                LARK_MAX_FILE_DOWNLOAD_BYTES / (1024 * 1024)
            );
        }
        Ok(bytes)
    }
    /// Send an image message by image_key.
    async fn send_image_msg(&self, chat_id: &str, image_key: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let content = serde_json::json!({ "image_key": image_key }).to_string();
        let body = serde_json::json!({
            "receive_id": chat_id,
            "msg_type": "image",
            "content": content,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            ensure_lark_send_success(rs, &rr, "image after token refresh")?;
            return Ok(());
        }
        ensure_lark_send_success(status, &response, "image")?;
        Ok(())
    }
    /// Upload a local file or download a URL, then send as image message.
    async fn send_lark_image(&self, chat_id: &str, target: &str) -> anyhow::Result<()> {
        let (bytes, filename) = if target.starts_with("http://") || target.starts_with("https://") {
            let resp = self.http_client().get(target).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("Lark: failed to fetch image URL: {target}");
            }
            let name = target.rsplit('/').next().unwrap_or("image.png").to_string();
            (resp.bytes().await?.to_vec(), name)
        } else {
            let path = std::path::Path::new(target);
            if !path.exists() {
                anyhow::bail!("Lark: image file not found: {target}");
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image.png")
                .to_string();
            (tokio::fs::read(path).await?, name)
        };
        let image_key = self.upload_image(bytes, &filename).await?;
        self.send_image_msg(chat_id, &image_key).await
    }
    /// Upload a file to Lark and return the `file_key`.
    async fn upload_file(
        &self,
        file_bytes: Vec<u8>,
        filename: &str,
        file_type: &str,
    ) -> anyhow::Result<String> {
        if file_bytes.len() > LARK_MAX_FILE_UPLOAD_BYTES {
            anyhow::bail!(
                "Lark: file exceeds max upload size ({} bytes > {LARK_MAX_FILE_UPLOAD_BYTES})",
                file_bytes.len()
            );
        }
        let token = self.get_tenant_access_token().await?;
        let url = self.upload_file_url();
        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")?;
        let form = reqwest::multipart::Form::new()
            .text("file_type", file_type.to_string())
            .text("file_name", filename.to_string())
            .part("file", part);
        let resp = self
            .http_client()
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(form)
            .send()
            .await?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await?;
        if !status.is_success() {
            anyhow::bail!("Lark upload_file failed: status={status}, body={body}");
        }
        let code = extract_lark_response_code(&body).unwrap_or(-1);
        if code != 0 {
            anyhow::bail!("Lark upload_file failed: code={code}, body={body}");
        }
        body.pointer("/data/file_key")
            .and_then(|k| k.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Lark upload_file: missing file_key in response"))
    }
    /// Send a file message by file_key.
    async fn send_file_msg(&self, chat_id: &str, file_key: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let content = serde_json::json!({ "file_key": file_key }).to_string();
        let body = serde_json::json!({
            "receive_id": chat_id,
            "msg_type": "file",
            "content": content,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            ensure_lark_send_success(rs, &rr, "file after token refresh")?;
            return Ok(());
        }
        ensure_lark_send_success(status, &response, "file")?;
        Ok(())
    }
    /// Send an audio message by file_key.
    async fn send_audio_msg(&self, chat_id: &str, file_key: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let content = serde_json::json!({ "file_key": file_key }).to_string();
        let body = serde_json::json!({
            "receive_id": chat_id,
            "msg_type": "audio",
            "content": content,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            ensure_lark_send_success(rs, &rr, "audio after token refresh")?;
            return Ok(());
        }
        ensure_lark_send_success(status, &response, "audio")?;
        Ok(())
    }
    /// Send a media (video) message by file_key.
    async fn send_media_msg(&self, chat_id: &str, file_key: &str) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let content = serde_json::json!({ "file_key": file_key }).to_string();
        let body = serde_json::json!({
            "receive_id": chat_id,
            "msg_type": "media",
            "content": content,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            ensure_lark_send_success(rs, &rr, "media after token refresh")?;
            return Ok(());
        }
        ensure_lark_send_success(status, &response, "media")?;
        Ok(())
    }
    /// Upload a local file or download a URL, then send as the appropriate message type.
    async fn send_lark_attachment(
        &self,
        chat_id: &str,
        attachment: &LarkAttachment,
    ) -> anyhow::Result<()> {
        let target = attachment.target.trim();
        let (bytes, filename) = if target.starts_with("http://") || target.starts_with("https://") {
            let resp = self.http_client().get(target).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("Lark: failed to fetch attachment URL: {target}");
            }
            let name = target.rsplit('/').next().unwrap_or("file").to_string();
            (resp.bytes().await?.to_vec(), name)
        } else {
            let path = std::path::Path::new(target);
            if !path.exists() {
                anyhow::bail!("Lark: attachment file not found: {target}");
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            (tokio::fs::read(path).await?, name)
        };
        let ext = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let file_type = resolve_feishu_file_type(ext);
        let file_key = self.upload_file(bytes, &filename, file_type).await?;
        match attachment.kind {
            LarkAttachmentKind::Image => self.send_image_msg(chat_id, &file_key).await,
            LarkAttachmentKind::Document => self.send_file_msg(chat_id, &file_key).await,
            LarkAttachmentKind::Audio => self.send_audio_msg(chat_id, &file_key).await,
            LarkAttachmentKind::Video => self.send_media_msg(chat_id, &file_key).await,
        }
    }
    async fn send_text_once(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> anyhow::Result<(reqwest::StatusCode, serde_json::Value)> {
        let resp = self
            .http_client()
            .post(url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let raw = resp.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "raw": raw }));
        Ok((status, parsed))
    }

    /// Parse an event callback payload and extract text messages
    pub fn parse_event_payload(&self, payload: &serde_json::Value) -> Vec<ChannelMessage> {
        let mut messages = Vec::new();

        // Lark event v2 structure:
        // { "header": { "event_type": "im.message.receive_v1" }, "event": { "message": { ... }, "sender": { ... } } }
        let event_type = payload
            .pointer("/header/event_type")
            .and_then(|e| e.as_str())
            .unwrap_or("");

        if event_type != "im.message.receive_v1" {
            return messages;
        }

        let event = match payload.get("event") {
            Some(e) => e,
            None => return messages,
        };

        // Extract sender open_id
        let open_id = event
            .pointer("/sender/sender_id/open_id")
            .and_then(|s| s.as_str())
            .unwrap_or("");

        if open_id.is_empty() {
            return messages;
        }

        // Check allowlist
        if !self.is_user_allowed(open_id) {
            tracing::warn!("Lark: ignoring message from unauthorized user: {open_id}");
            return messages;
        }
        // Auto-share docs_sync documents with new users
        #[cfg(feature = "feishu-docs-sync")]
        if let Some(ref sharer) = self.docs_sharer {
            let sharer = std::sync::Arc::clone(sharer);
            let oid = open_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = sharer.share_all_docs_with(&oid).await {
                    tracing::warn!("docs_sync: auto-share failed for {oid}: {e}");
                }
            });
        }

        // Extract message content (text and post supported)
        let msg_type = event
            .pointer("/message/message_type")
            .and_then(|t| t.as_str())
            .unwrap_or("");

        let content_str = event
            .pointer("/message/content")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        let text: String = match msg_type {
            "text" => {
                let extracted = serde_json::from_str::<serde_json::Value>(content_str)
                    .ok()
                    .and_then(|v| {
                        v.get("text")
                            .and_then(|t| t.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                    });
                match extracted {
                    Some(t) => t,
                    None => return messages,
                }
            }
            "post" => match parse_post_content(content_str) {
                Some(t) => t,
                None => return messages,
            },
            "image" => match extract_image_key(content_str) {
                Some(key) => format!("[IMAGE:lark_image_key:{key}]"),
                None => return messages,
            },
            "file" => match extract_file_key_and_name(content_str) {
                Some((key, name)) => {
                    let label = name.as_deref().unwrap_or(&key);
                    format!("[DOCUMENT:lark_file_key:{key}:{label}]")
                }
                None => return messages,
            },
            "audio" => match extract_file_key(content_str) {
                Some(key) => format!("[AUDIO:lark_file_key:{key}]"),
                None => return messages,
            },
            "media" => match extract_file_key_and_name(content_str) {
                Some((key, name)) => {
                    let label = name.as_deref().unwrap_or(&key);
                    format!("[VIDEO:lark_file_key:{key}:{label}]")
                }
                None => return messages,
            },
            _ => return messages,
        };

        // Strip @_user_N placeholders (parity with WS path)
        let text = strip_at_placeholders(&text);
        let text = text.trim().to_string();
        if text.is_empty() {
            return messages;
        }

        // Group-chat: only respond when explicitly @-mentioned
        let chat_type = event
            .pointer("/message/chat_type")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        let mentions: Vec<serde_json::Value> = event
            .pointer("/message/mentions")
            .and_then(|m| serde_json::from_value(m.clone()).ok())
            .unwrap_or_default();
        if chat_type == "group" && !should_respond_in_group(&mentions) {
            return messages;
        }
        let timestamp = event
            .pointer("/message/create_time")
            .and_then(|t| t.as_str())
            .and_then(|t| t.parse::<u64>().ok())
            // Lark timestamps are in milliseconds
            .map(|ms| ms / 1000)
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            });

        let chat_id = event
            .pointer("/message/chat_id")
            .and_then(|c| c.as_str())
            .unwrap_or(open_id);

        messages.push(ChannelMessage {
            id: Uuid::new_v4().to_string(),
            sender: chat_id.to_string(),
            reply_target: chat_id.to_string(),
            content: text,
            channel: self.channel_name().to_string(),
            timestamp,
            thread_ts: None,
        });

        messages
    }

    // ── CardKit streaming helpers ──────────────────────────────────────────

    /// Create a new CardKit card entity and return its `card_id`.
    async fn create_card(&self, content_json: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let url = self.cardkit_url();
        let body = serde_json::json!({
            "type": "card_json",
            "data": content_json,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            ensure_lark_send_success(rs, &rr, "create_card after token refresh")?;
            return rr
                .pointer("/data/card_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| anyhow::anyhow!("Lark create_card: missing card_id in response"));
        }
        ensure_lark_send_success(status, &response, "create_card")?;
        response
            .pointer("/data/card_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Lark create_card: missing card_id in response"))
    }
    /// Update an existing CardKit card entity with new content.
    async fn update_card(&self, card_id: &str, content_json: &str, sequence: u64) -> anyhow::Result<()> {
        let token = self.get_tenant_access_token().await?;
        let url = format!("{}/{card_id}", self.cardkit_url());
        let body = serde_json::json!({
            "type": "card_json",
            "data": content_json,
            "sequence": sequence,
        });
        let resp = self
            .http_client()
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let raw = resp.text().await.unwrap_or_default();
        let parsed = serde_json::from_str::<serde_json::Value>(&raw)
            .unwrap_or_else(|_| serde_json::json!({ "raw": raw }));
        if should_refresh_lark_tenant_token(status, &parsed) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let resp2 = self
                .http_client()
                .put(&url)
                .header("Authorization", format!("Bearer {new_token}"))
                .header("Content-Type", "application/json; charset=utf-8")
                .json(&body)
                .send()
                .await?;
            let rs = resp2.status();
            let rr: serde_json::Value = resp2.json().await.unwrap_or_default();
            ensure_lark_send_success(rs, &rr, "update_card after token refresh")?;
            return Ok(());
        }
        ensure_lark_send_success(status, &parsed, "update_card")?;
        Ok(())
    }
    /// Send a card message referencing an existing CardKit card_id. Returns message_id.
    async fn send_card_message(&self, recipient: &str, card_id: &str) -> anyhow::Result<String> {
        let token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let content = serde_json::json!({
            "type": "card",
            "data": {
                "card_id": card_id
            }
        })
        .to_string();
        let body = serde_json::json!({
            "receive_id": recipient,
            "msg_type": "interactive",
            "content": content,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            ensure_lark_send_success(rs, &rr, "send_card_message after token refresh")?;
            return rr
                .pointer("/data/message_id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| anyhow::anyhow!("Lark send_card_message: missing message_id"));
        }
        ensure_lark_send_success(status, &response, "send_card_message")?;
        response
            .pointer("/data/message_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| anyhow::anyhow!("Lark send_card_message: missing message_id"))
    }

    // ── Typing indicator helpers ───────────────────────────────────────────

    /// Show a "正在处理..." CardKit card as a typing indicator.
    /// Graceful no-op when CardKit is unavailable.
    pub async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let card_json = serde_json::json!({
            "schema": "2.0",
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "content": "⏳ 正在处理..."
                }]
            }
        })
        .to_string();

        let card_id = match self.create_card(&card_json).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("[{}] start_typing: CardKit unavailable, skipping: {e}", self.channel_name());
                return Ok(());
            }
        };

        if let Err(e) = self.send_card_message(recipient, &card_id).await {
            tracing::warn!("[{}] start_typing: failed to send typing card: {e}", self.channel_name());
            return Ok(());
        }

        self.typing_card_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(recipient.to_string(), card_id);

        Ok(())
    }

    /// Remove the typing indicator card for a recipient.
    /// Graceful no-op when no typing card exists or CardKit is unavailable.
    pub async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let card_id = self
            .typing_card_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(recipient);

        if let Some(card_id) = card_id {
            // Update the card to a blank state so the indicator disappears.
            let empty_json = serde_json::json!({
                "schema": "2.0",
                "body": { "elements": [] }
            })
            .to_string();

            if let Err(e) = self.update_card(&card_id, &empty_json, 2).await {
                tracing::warn!(
                    "[{}] stop_typing: failed to clear typing card {card_id}: {e}",
                    self.channel_name()
                );
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Channel for LarkChannel {
    fn name(&self) -> &str {
        self.channel_name()
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let (text, attachments) = parse_lark_attachment_markers(&message.content);
        for att in &attachments {
            let result = match att.kind {
                LarkAttachmentKind::Image => {
                    self.send_lark_image(&message.recipient, &att.target).await
                }
                _ => self.send_lark_attachment(&message.recipient, att).await,
            };
            if let Err(e) = result {
                tracing::warn!(
                    "Lark: {:?} send failed for {}: {e}",
                    att.kind,
                    att.target
                );
            }
        }
        if text.is_empty() {
            return Ok(());
        }
        let token = self.get_tenant_access_token().await?;
        let url = self.send_message_url();
        let content = serde_json::json!({
            "elements": [{
                "tag": "markdown",
                "content": text
            }]
        })
        .to_string();
        let body = serde_json::json!({
            "receive_id": message.recipient,
            "msg_type": "interactive",
            "content": content,
        });
        let (status, response) = self.send_text_once(&url, &token, &body).await?;
        if should_refresh_lark_tenant_token(status, &response) {
            self.invalidate_token().await;
            let new_token = self.get_tenant_access_token().await?;
            let (rs, rr) = self.send_text_once(&url, &new_token, &body).await?;
            if should_refresh_lark_tenant_token(rs, &rr) {
                anyhow::bail!("Lark send failed after token refresh: status={rs}, body={rr}");
            }
            ensure_lark_send_success(rs, &rr, "after token refresh")?;
            return Ok(());
        }
        ensure_lark_send_success(status, &response, "without token refresh")?;
        Ok(())
    }

    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        use crate::config::schema::LarkReceiveMode;
        match self.receive_mode {
            LarkReceiveMode::Websocket => self.listen_ws(tx).await,
            LarkReceiveMode::Webhook => self.listen_http(tx).await,
        }
    }

    async fn health_check(&self) -> bool {
        self.get_tenant_access_token().await.is_ok()
    }
    fn supports_draft_updates(&self) -> bool {
        self.stream_mode != StreamMode::Off
    }

    async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
        if self.stream_mode == StreamMode::Off {
            return Ok(None);
        }

        let initial_text = if message.content.is_empty() {
            "...".to_string()
        } else {
            message.content.clone()
        };

        let card_json = serde_json::json!({
            "schema": "2.0",
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "content": initial_text
                }]
            }
        })
        .to_string();

        let card_id = match self.create_card(&card_json).await {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!("Lark CardKit create_card failed, falling back to send(): {e}");
                return Ok(None);
            }
        };

        match self.send_card_message(&message.recipient, &card_id).await {
            Ok(_msg_id) => {
                self.card_sequence
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(card_id.clone(), 1);
                self.last_draft_update
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(card_id.clone(), Instant::now());
                Ok(Some(card_id))
            }
            Err(e) => {
                tracing::warn!("Lark CardKit send_card_message failed: {e}");
                Ok(None)
            }
        }
    }
    async fn update_draft(
        &self,
        _recipient: &str,
        draft_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        // Throttle: skip update if too soon since last one
        {
            let last_updates = self.last_draft_update.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(last_time) = last_updates.get(draft_id) {
                let elapsed = u64::try_from(last_time.elapsed().as_millis()).unwrap_or(u64::MAX);
                if elapsed < self.draft_update_interval_ms.max(500) {
                    return Ok(());
                }
            }
        }
        let sequence = {
            let mut seqs = self.card_sequence.lock().unwrap_or_else(|e| e.into_inner());
            let seq = seqs.entry(draft_id.to_string()).or_insert(1);
            *seq += 1;
            *seq
        };
        let card_json = serde_json::json!({
            "schema": "2.0",
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "content": text
                }]
            }
        })
        .to_string();
        if let Err(e) = self.update_card(draft_id, &card_json, sequence).await {
            tracing::warn!("Lark CardKit update_card failed (non-fatal): {e}");
        } else {
            self.last_draft_update
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(draft_id.to_string(), Instant::now());
        }
        Ok(())
    }
    async fn finalize_draft(
        &self,
        _recipient: &str,
        draft_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let sequence = {
            let mut seqs = self.card_sequence.lock().unwrap_or_else(|e| e.into_inner());
            let seq = seqs.entry(draft_id.to_string()).or_insert(1);
            *seq += 1;
            *seq
        };
        let card_json = serde_json::json!({
            "schema": "2.0",
            "body": {
                "elements": [{
                    "tag": "markdown",
                    "content": text
                }]
            }
        })
        .to_string();
        if let Err(e) = self.update_card(draft_id, &card_json, sequence).await {
            tracing::warn!("Lark CardKit finalize_draft failed: {e}");
        }
        // Clean up tracking state
        self.card_sequence
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(draft_id);
        self.last_draft_update
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(draft_id);
        Ok(())
    }
    async fn cancel_draft(&self, _recipient: &str, draft_id: &str) -> anyhow::Result<()> {
        self.card_sequence
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(draft_id);
        self.last_draft_update
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(draft_id);
        Ok(())
    }
}

impl LarkChannel {
    /// HTTP callback server (legacy — requires a public endpoint).
    /// Use `listen()` (WS long-connection) for new deployments.
    pub async fn listen_http(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        use axum::{extract::State, routing::post, Json, Router};

        #[derive(Clone)]
        struct AppState {
            verification_token: String,
            channel: Arc<LarkChannel>,
            tx: tokio::sync::mpsc::Sender<ChannelMessage>,
        }

        async fn handle_event(
            State(state): State<AppState>,
            Json(payload): Json<serde_json::Value>,
        ) -> axum::response::Response {
            use axum::http::StatusCode;
            use axum::response::IntoResponse;

            // URL verification challenge
            if let Some(challenge) = payload.get("challenge").and_then(|c| c.as_str()) {
                // Verify token if present
                let token_ok = payload
                    .get("token")
                    .and_then(|t| t.as_str())
                    .map_or(true, |t| t == state.verification_token);

                if !token_ok {
                    return (StatusCode::FORBIDDEN, "invalid token").into_response();
                }

                let resp = serde_json::json!({ "challenge": challenge });
                return (StatusCode::OK, Json(resp)).into_response();
            }

            // Parse event messages
            let messages = state.channel.parse_event_payload(&payload);
            if !messages.is_empty() {
                if let Some(message_id) = payload
                    .pointer("/event/message/message_id")
                    .and_then(|m| m.as_str())
                {
                    let ack_emoji = "OK";
                    let reaction_channel = Arc::clone(&state.channel);
                    let reaction_message_id = message_id.to_string();
                    tokio::spawn(async move {
                        reaction_channel
                            .try_add_ack_reaction(&reaction_message_id, &ack_emoji)
                            .await;
                    });
                }
            }

            for msg in messages {
                if state.tx.send(msg).await.is_err() {
                    tracing::warn!("Lark: message channel closed");
                    break;
                }
            }

            (StatusCode::OK, "ok").into_response()
        }

        let port = self.port.ok_or_else(|| {
            anyhow::anyhow!("Lark webhook mode requires `port` to be set in [channels_config.lark]")
        })?;

        let state = AppState {
            verification_token: self.verification_token.clone(),
            channel: Arc::new(self.clone()),
            tx,
        };

        let app = Router::new()
            .route("/lark", post(handle_event))
            .with_state(state);

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        tracing::info!("Lark event callback server listening on {addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WS helper functions
// ─────────────────────────────────────────────────────────────────────────────


/// Flatten a Feishu `post` rich-text message to plain text.
///
/// Returns `None` when the content cannot be parsed or yields no usable text,
/// so callers can simply `continue` rather than forwarding a meaningless
/// placeholder string to the agent.
fn parse_post_content(content: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let locale = parsed
        .get("zh_cn")
        .or_else(|| parsed.get("en_us"))
        .or_else(|| {
            parsed
                .as_object()
                .and_then(|m| m.values().find(|v| v.is_object()))
        })?;

    let mut text = String::new();

    if let Some(title) = locale
        .get("title")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
    {
        text.push_str(title);
        text.push_str("\n\n");
    }

    if let Some(paragraphs) = locale.get("content").and_then(|c| c.as_array()) {
        for para in paragraphs {
            if let Some(elements) = para.as_array() {
                for el in elements {
                    match el.get("tag").and_then(|t| t.as_str()).unwrap_or("") {
                        "text" => {
                            if let Some(t) = el.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                        "a" => {
                            text.push_str(
                                el.get("text")
                                    .and_then(|t| t.as_str())
                                    .filter(|s| !s.is_empty())
                                    .or_else(|| el.get("href").and_then(|h| h.as_str()))
                                    .unwrap_or(""),
                            );
                        }
                        "at" => {
                            let n = el
                                .get("user_name")
                                .and_then(|n| n.as_str())
                                .or_else(|| el.get("user_id").and_then(|i| i.as_str()))
                                .unwrap_or("user");
                            text.push('@');
                            text.push_str(n);
                        }
                        _ => {}
                    }
                }
                text.push('\n');
            }
        }
    }

    let result = text.trim().to_string();
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
/// Extract `image_key` from a Lark image message content JSON.
fn extract_image_key(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("image_key")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Extract `file_key` from a Lark file/audio message content JSON.
fn extract_file_key(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("file_key")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Extract `file_key` and optional `file_name` from a Lark file/media message content JSON.
fn extract_file_key_and_name(content: &str) -> Option<(String, Option<String>)> {
    let v = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let key = v.get("file_key")?.as_str().filter(|s| !s.is_empty())?.to_string();
    let name = v
        .get("file_name")
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Some((key, name))
}

/// Remove `@_user_N` placeholder tokens injected by Feishu in group chats.
fn strip_at_placeholders(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        if ch == '@' {
            let rest: String = chars.clone().map(|(_, c)| c).collect();
            if let Some(after) = rest.strip_prefix("_user_") {
                let skip =
                    "_user_".len() + after.chars().take_while(|c| c.is_ascii_digit()).count();
                for _ in 0..skip {
                    chars.next();
                }
                if chars.peek().map(|(_, c)| *c == ' ').unwrap_or(false) {
                    chars.next();
                }
                continue;
            }
        }
        result.push(ch);
    }
    result
}

/// In group chats, only respond when the bot is explicitly @-mentioned.
/// Feishu bot mentions have an empty `user_id` in the `id` object, distinguishing
/// them from regular user mentions.
fn should_respond_in_group(mentions: &[serde_json::Value]) -> bool {
    mentions.iter().any(|m| {
        // Bot mentions: id.user_id is empty or absent
        let user_id = m
            .pointer("/id/user_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        user_id.is_empty()
    })
}

/// Map file extension to Feishu upload `file_type` parameter.
fn resolve_feishu_file_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => "pdf",
        "doc" | "docx" => "doc",
        "xls" | "xlsx" => "xls",
        "ppt" | "pptx" => "ppt",
        "opus" | "ogg" => "opus",
        "mp4" => "mp4",
        _ => "stream",
    }
}

/// A parsed attachment marker from outgoing message content.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LarkAttachment {
    kind: LarkAttachmentKind,
    target: String,
}

/// Extract attachment markers (`[IMAGE:…]`, `[DOCUMENT:…]`, `[AUDIO:…]`, `[VIDEO:…]`) from
/// outgoing message content. Returns (cleaned_text, attachments).
fn parse_lark_attachment_markers(message: &str) -> (String, Vec<LarkAttachment>) {
    let mut cleaned = String::with_capacity(message.len());
    let mut attachments = Vec::new();
    let mut cursor = 0;
    while cursor < message.len() {
        let Some(open_rel) = message[cursor..].find('[') else {
            cleaned.push_str(&message[cursor..]);
            break;
        };
        let open = cursor + open_rel;
        cleaned.push_str(&message[cursor..open]);
        let Some(close_rel) = message[open..].find(']') else {
            cleaned.push_str(&message[open..]);
            break;
        };
        let close = open + close_rel;
        let marker = &message[open + 1..close];
        if let Some((kind_str, target)) = marker.split_once(':') {
            let target = target.trim();
            let kind = match kind_str.trim().to_ascii_uppercase().as_str() {
                "IMAGE" => Some(LarkAttachmentKind::Image),
                "DOCUMENT" => Some(LarkAttachmentKind::Document),
                "AUDIO" => Some(LarkAttachmentKind::Audio),
                "VIDEO" => Some(LarkAttachmentKind::Video),
                _ => None,
            };
            if let Some(kind) = kind {
                if !target.is_empty() {
                    attachments.push(LarkAttachment {
                        kind,
                        target: target.to_string(),
                    });
                    cursor = close + 1;
                    continue;
                }
            }
        }
        cleaned.push_str(&message[open..=close]);
        cursor = close + 1;
    }
    (cleaned.trim().to_string(), attachments)
}

/// Legacy wrapper: extract only `[IMAGE:…]` markers (used by existing tests).
fn parse_lark_image_markers(message: &str) -> (String, Vec<String>) {
    let (text, attachments) = parse_lark_attachment_markers(message);
    let images = attachments
        .into_iter()
        .filter(|a| a.kind == LarkAttachmentKind::Image)
        .map(|a| a.target)
        .collect();
    (text, images)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_channel() -> LarkChannel {
        LarkChannel::new(
            "cli_test_app_id".into(),
            "test_app_secret".into(),
            "test_verification_token".into(),
            None,
            vec!["ou_testuser123".into()],
        )
    }

    #[test]
    fn lark_channel_name() {
        let ch = make_channel();
        assert_eq!(ch.name(), "lark");
    }


    #[test]
    fn lark_should_refresh_token_on_http_401() {
        let body = serde_json::json!({ "code": 0 });
        assert!(should_refresh_lark_tenant_token(
            reqwest::StatusCode::UNAUTHORIZED,
            &body
        ));
    }

    #[test]
    fn lark_should_refresh_token_on_body_code_99991663() {
        let body = serde_json::json!({
            "code": LARK_INVALID_ACCESS_TOKEN_CODE,
            "msg": "Invalid access token for authorization."
        });
        assert!(should_refresh_lark_tenant_token(
            reqwest::StatusCode::OK,
            &body
        ));
    }

    #[test]
    fn lark_should_not_refresh_token_on_success_body() {
        let body = serde_json::json!({ "code": 0, "msg": "ok" });
        assert!(!should_refresh_lark_tenant_token(
            reqwest::StatusCode::OK,
            &body
        ));
    }

    #[test]
    fn lark_extract_token_ttl_seconds_supports_expire_and_expires_in() {
        let body_expire = serde_json::json!({ "expire": 7200 });
        let body_expires_in = serde_json::json!({ "expires_in": 3600 });
        let body_missing = serde_json::json!({});
        assert_eq!(extract_lark_token_ttl_seconds(&body_expire), 7200);
        assert_eq!(extract_lark_token_ttl_seconds(&body_expires_in), 3600);
        assert_eq!(
            extract_lark_token_ttl_seconds(&body_missing),
            LARK_DEFAULT_TOKEN_TTL.as_secs()
        );
    }

    #[test]
    fn lark_next_token_refresh_deadline_reserves_refresh_skew() {
        let now = Instant::now();
        let regular = next_token_refresh_deadline(now, 7200);
        let short_ttl = next_token_refresh_deadline(now, 60);

        assert_eq!(regular.duration_since(now), Duration::from_secs(7080));
        assert_eq!(short_ttl.duration_since(now), Duration::from_secs(1));
    }

    #[test]
    fn lark_ensure_send_success_rejects_non_zero_code() {
        let ok = serde_json::json!({ "code": 0 });
        let bad = serde_json::json!({ "code": 12345, "msg": "bad request" });

        assert!(ensure_lark_send_success(reqwest::StatusCode::OK, &ok, "test").is_ok());
        assert!(ensure_lark_send_success(reqwest::StatusCode::OK, &bad, "test").is_err());
    }

    #[test]
    fn lark_user_allowed_exact() {
        let ch = make_channel();
        assert!(ch.is_user_allowed("ou_testuser123"));
        assert!(!ch.is_user_allowed("ou_other"));
    }

    #[test]
    fn lark_user_allowed_wildcard() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        assert!(ch.is_user_allowed("ou_anyone"));
    }

    #[test]
    fn lark_user_denied_empty() {
        let ch = LarkChannel::new("id".into(), "secret".into(), "token".into(), None, vec![]);
        assert!(!ch.is_user_allowed("ou_anyone"));
    }

    #[test]
    fn lark_parse_challenge() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "challenge": "abc123",
            "token": "test_verification_token",
            "type": "url_verification"
        });
        // Challenge payloads should not produce messages
        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_valid_text_message() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_testuser123"
                    }
                },
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"Hello ZeroClaw!\"}",
                    "chat_id": "oc_chat123",
                    "create_time": "1699999999000"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello ZeroClaw!");
        assert_eq!(msgs[0].sender, "oc_chat123");
        assert_eq!(msgs[0].channel, "lark");
        assert_eq!(msgs[0].timestamp, 1_699_999_999);
    }

    #[test]
    fn lark_parse_unauthorized_user() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_unauthorized" } },
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"spam\"}",
                    "chat_id": "oc_chat",
                    "create_time": "1000"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_image_message_produces_marker() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "image",
                    "content": "{\"image_key\":\"img_v3_test_key\"}",
                    "chat_id": "oc_chat"
                }
            }
        });
        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "[IMAGE:lark_image_key:img_v3_test_key]");
    }
    #[test]
    fn lark_parse_image_message_empty_key_skipped() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "image",
                    "content": "{}",
                    "chat_id": "oc_chat"
                }
            }
        });
        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_empty_text_skipped() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"\"}",
                    "chat_id": "oc_chat"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_wrong_event_type() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.chat.disbanded_v1" },
            "event": {}
        });

        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_missing_sender() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"hello\"}",
                    "chat_id": "oc_chat"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_unicode_message() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"Hello world 🌍\"}",
                    "chat_id": "oc_chat",
                    "create_time": "1000"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "Hello world 🌍");
    }

    #[test]
    fn lark_parse_missing_event() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_parse_invalid_content_json() {
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "text",
                    "content": "not valid json",
                    "chat_id": "oc_chat"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert!(msgs.is_empty());
    }

    #[test]
    fn lark_config_serde() {
        use crate::config::schema::{LarkConfig, LarkReceiveMode};
        let lc = LarkConfig {
            app_id: "cli_app123".into(),
            app_secret: "secret456".into(),
            encrypt_key: None,
            verification_token: Some("vtoken789".into()),
            allowed_users: vec!["ou_user1".into(), "ou_user2".into()],
            use_feishu: false,
            receive_mode: LarkReceiveMode::default(),
            port: None,
        };
        let json = serde_json::to_string(&lc).unwrap();
        let parsed: LarkConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.app_id, "cli_app123");
        assert_eq!(parsed.app_secret, "secret456");
        assert_eq!(parsed.verification_token.as_deref(), Some("vtoken789"));
        assert_eq!(parsed.allowed_users.len(), 2);
    }

    #[test]
    fn lark_config_toml_roundtrip() {
        use crate::config::schema::{LarkConfig, LarkReceiveMode};
        let lc = LarkConfig {
            app_id: "app".into(),
            app_secret: "secret".into(),
            encrypt_key: None,
            verification_token: Some("tok".into()),
            allowed_users: vec!["*".into()],
            use_feishu: false,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
        };
        let toml_str = toml::to_string(&lc).unwrap();
        let parsed: LarkConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.app_id, "app");
        assert_eq!(parsed.verification_token.as_deref(), Some("tok"));
        assert_eq!(parsed.allowed_users, vec!["*"]);
    }

    #[test]
    fn lark_config_defaults_optional_fields() {
        use crate::config::schema::{LarkConfig, LarkReceiveMode};
        let json = r#"{"app_id":"a","app_secret":"s"}"#;
        let parsed: LarkConfig = serde_json::from_str(json).unwrap();
        assert!(parsed.verification_token.is_none());
        assert!(parsed.allowed_users.is_empty());
        assert_eq!(parsed.receive_mode, LarkReceiveMode::Websocket);
        assert!(parsed.port.is_none());
    }

    #[test]
    fn lark_from_config_preserves_mode_and_region() {
        use crate::config::schema::{LarkConfig, LarkReceiveMode};

        let cfg = LarkConfig {
            app_id: "cli_app123".into(),
            app_secret: "secret456".into(),
            encrypt_key: None,
            verification_token: Some("vtoken789".into()),
            allowed_users: vec!["*".into()],
            use_feishu: false,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
        };

        let ch = LarkChannel::from_config(&cfg);

        assert_eq!(ch.api_base(), LARK_BASE_URL);
        assert_eq!(ch.ws_base(), LARK_WS_BASE_URL);
        assert_eq!(ch.receive_mode, LarkReceiveMode::Webhook);
        assert_eq!(ch.port, Some(9898));
    }

    #[test]
    fn lark_from_lark_config_ignores_legacy_feishu_flag() {
        use crate::config::schema::{LarkConfig, LarkReceiveMode};

        let cfg = LarkConfig {
            app_id: "cli_app123".into(),
            app_secret: "secret456".into(),
            encrypt_key: None,
            verification_token: Some("vtoken789".into()),
            allowed_users: vec!["*".into()],
            use_feishu: true,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
        };

        let ch = LarkChannel::from_lark_config(&cfg);

        assert_eq!(ch.api_base(), LARK_BASE_URL);
        assert_eq!(ch.ws_base(), LARK_WS_BASE_URL);
        assert_eq!(ch.name(), "lark");
    }

    #[test]
    fn lark_from_feishu_config_sets_feishu_platform() {
        use crate::config::schema::{FeishuConfig, LarkReceiveMode};

        let cfg = FeishuConfig {
            app_id: "cli_feishu_app123".into(),
            app_secret: "secret456".into(),
            encrypt_key: None,
            verification_token: Some("vtoken789".into()),
            allowed_users: vec!["*".into()],
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(9898),
        };

        let ch = LarkChannel::from_feishu_config(&cfg);

        assert_eq!(ch.api_base(), FEISHU_BASE_URL);
        assert_eq!(ch.ws_base(), FEISHU_WS_BASE_URL);
        assert_eq!(ch.name(), "feishu");
    }

    #[test]
    fn lark_parse_fallback_sender_to_open_id() {
        // When chat_id is missing, sender should fall back to open_id
        let ch = LarkChannel::new(
            "id".into(),
            "secret".into(),
            "token".into(),
            None,
            vec!["*".into()],
        );
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "text",
                    "content": "{\"text\":\"hello\"}",
                    "create_time": "1000"
                }
            }
        });

        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].sender, "ou_user");
    }

    #[test]
    fn lark_reaction_url_matches_region() {
        let ch_lark = make_channel();
        assert_eq!(
            ch_lark.message_reaction_url("om_test_message_id"),
            "https://open.larksuite.com/open-apis/im/v1/messages/om_test_message_id/reactions"
        );

        let feishu_cfg = crate::config::schema::FeishuConfig {
            app_id: "cli_app123".into(),
            app_secret: "secret456".into(),
            encrypt_key: None,
            verification_token: Some("vtoken789".into()),
            allowed_users: vec!["*".into()],
            receive_mode: crate::config::schema::LarkReceiveMode::Webhook,
            port: Some(9898),
        };
        let ch_feishu = LarkChannel::from_feishu_config(&feishu_cfg);
        assert_eq!(
            ch_feishu.message_reaction_url("om_test_message_id"),
            "https://open.feishu.cn/open-apis/im/v1/messages/om_test_message_id/reactions"
        );
    }

    #[test]
    fn parse_lark_image_markers_extracts_images() {
        let msg = "Here is an image [IMAGE:/tmp/a.png] and text";
        let (cleaned, images) = parse_lark_image_markers(msg);
        assert_eq!(cleaned, "Here is an image  and text");
        assert_eq!(images, vec!["/tmp/a.png"]);
    }
    #[test]
    fn parse_lark_image_markers_url() {
        let msg = "[IMAGE:https://example.com/img.jpg]";
        let (cleaned, images) = parse_lark_image_markers(msg);
        assert!(cleaned.is_empty());
        assert_eq!(images, vec!["https://example.com/img.jpg"]);
    }
    #[test]
    fn parse_lark_image_markers_no_markers() {
        let msg = "plain text [BOLD:test]";
        let (cleaned, images) = parse_lark_image_markers(msg);
        assert_eq!(cleaned, "plain text [BOLD:test]");
        assert!(images.is_empty());
    }
    #[test]
    fn extract_image_key_valid() {
        assert_eq!(
            extract_image_key(r#"{"image_key":"img_v3_abc"}"#),
            Some("img_v3_abc".to_string())
        );
    }
    #[test]
    fn extract_image_key_missing() {
        assert_eq!(extract_image_key("{}"), None);
        assert_eq!(extract_image_key("not json"), None);
    }
    #[test]
    fn parse_lark_attachment_markers_all_types() {
        let msg = "Hello [IMAGE:/tmp/a.png] world [DOCUMENT:/tmp/b.pdf] foo [AUDIO:/tmp/c.opus] bar [VIDEO:/tmp/d.mp4]";
        let (cleaned, attachments) = parse_lark_attachment_markers(msg);
        assert_eq!(cleaned, "Hello  world  foo  bar");
        assert_eq!(attachments.len(), 4);
        assert_eq!(attachments[0].kind, LarkAttachmentKind::Image);
        assert_eq!(attachments[0].target, "/tmp/a.png");
        assert_eq!(attachments[1].kind, LarkAttachmentKind::Document);
        assert_eq!(attachments[1].target, "/tmp/b.pdf");
        assert_eq!(attachments[2].kind, LarkAttachmentKind::Audio);
        assert_eq!(attachments[2].target, "/tmp/c.opus");
        assert_eq!(attachments[3].kind, LarkAttachmentKind::Video);
        assert_eq!(attachments[3].target, "/tmp/d.mp4");
    }
    #[test]
    fn parse_lark_attachment_markers_case_insensitive() {
        let msg = "[document:/tmp/a.pdf] [Audio:/tmp/b.ogg] [VIDEO:/tmp/c.mp4]";
        let (cleaned, attachments) = parse_lark_attachment_markers(msg);
        assert!(cleaned.is_empty());
        assert_eq!(attachments.len(), 3);
        assert_eq!(attachments[0].kind, LarkAttachmentKind::Document);
        assert_eq!(attachments[1].kind, LarkAttachmentKind::Audio);
        assert_eq!(attachments[2].kind, LarkAttachmentKind::Video);
    }
    #[test]
    fn parse_lark_attachment_markers_unknown_kind_preserved() {
        let msg = "text [UNKNOWN:/tmp/x] more";
        let (cleaned, attachments) = parse_lark_attachment_markers(msg);
        assert_eq!(cleaned, "text [UNKNOWN:/tmp/x] more");
        assert!(attachments.is_empty());
    }
    #[test]
    fn resolve_feishu_file_type_known_extensions() {
        assert_eq!(resolve_feishu_file_type("pdf"), "pdf");
        assert_eq!(resolve_feishu_file_type("doc"), "doc");
        assert_eq!(resolve_feishu_file_type("docx"), "doc");
        assert_eq!(resolve_feishu_file_type("xls"), "xls");
        assert_eq!(resolve_feishu_file_type("xlsx"), "xls");
        assert_eq!(resolve_feishu_file_type("ppt"), "ppt");
        assert_eq!(resolve_feishu_file_type("pptx"), "ppt");
        assert_eq!(resolve_feishu_file_type("opus"), "opus");
        assert_eq!(resolve_feishu_file_type("ogg"), "opus");
        assert_eq!(resolve_feishu_file_type("mp4"), "mp4");
    }
    #[test]
    fn resolve_feishu_file_type_unknown_falls_back_to_stream() {
        assert_eq!(resolve_feishu_file_type("zip"), "stream");
        assert_eq!(resolve_feishu_file_type("txt"), "stream");
        assert_eq!(resolve_feishu_file_type("png"), "stream");
        assert_eq!(resolve_feishu_file_type(""), "stream");
    }
    #[test]
    fn resolve_feishu_file_type_case_insensitive() {
        assert_eq!(resolve_feishu_file_type("PDF"), "pdf");
        assert_eq!(resolve_feishu_file_type("DOCX"), "doc");
        assert_eq!(resolve_feishu_file_type("Mp4"), "mp4");
    }
    #[test]
    fn lark_upload_file_url_matches_region() {
        let ch_lark = make_channel();
        assert_eq!(
            ch_lark.upload_file_url(),
            "https://open.larksuite.com/open-apis/im/v1/files"
        );

        let feishu_cfg = crate::config::schema::FeishuConfig {
            app_id: "cli_app123".into(),
            app_secret: "secret456".into(),
            encrypt_key: None,
            verification_token: Some("vtoken789".into()),
            allowed_users: vec!["*".into()],
            receive_mode: crate::config::schema::LarkReceiveMode::Webhook,
            port: Some(9898),
        };
        let ch_feishu = LarkChannel::from_feishu_config(&feishu_cfg);
        assert_eq!(
            ch_feishu.upload_file_url(),
            "https://open.feishu.cn/open-apis/im/v1/files"
        );
    }
    #[test]
    fn extract_file_key_valid() {
        assert_eq!(
            extract_file_key(r#"{"file_key":"file_v3_abc"}"#),
            Some("file_v3_abc".to_string())
        );
    }
    #[test]
    fn extract_file_key_missing() {
        assert_eq!(extract_file_key("{}"), None);
        assert_eq!(extract_file_key("not json"), None);
    }
    #[test]
    fn extract_file_key_and_name_with_name() {
        let content = r#"{"file_key":"file_v3_abc","file_name":"report.pdf"}"#;
        let result = extract_file_key_and_name(content);
        assert_eq!(result, Some(("file_v3_abc".to_string(), Some("report.pdf".to_string()))));
    }
    #[test]
    fn extract_file_key_and_name_without_name() {
        let content = r#"{"file_key":"file_v3_abc"}"#;
        let result = extract_file_key_and_name(content);
        assert_eq!(result, Some(("file_v3_abc".to_string(), None)));
    }
    #[test]
    fn extract_file_key_and_name_missing_key() {
        assert_eq!(extract_file_key_and_name("{}"), None);
    }
    #[test]
    fn parse_event_payload_file_message() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1", "event_id": "ev1" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "file",
                    "content": "{\"file_key\":\"file_v3_abc\",\"file_name\":\"report.pdf\"}",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_id": "om_msg1",
                    "create_time": "1000"
                }
            }
        });
        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "[DOCUMENT:lark_file_key:file_v3_abc:report.pdf]");
    }
    #[test]
    fn parse_event_payload_audio_message() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1", "event_id": "ev2" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "audio",
                    "content": "{\"file_key\":\"file_v3_audio\"}",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_id": "om_msg2",
                    "create_time": "2000"
                }
            }
        });
        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "[AUDIO:lark_file_key:file_v3_audio]");
    }
    #[test]
    fn parse_event_payload_media_message() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1", "event_id": "ev3" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "media",
                    "content": "{\"file_key\":\"file_v3_video\",\"file_name\":\"clip.mp4\"}",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_id": "om_msg3",
                    "create_time": "3000"
                }
            }
        });
        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "[VIDEO:lark_file_key:file_v3_video:clip.mp4]");
    }
    #[test]
    fn parse_event_payload_file_message_no_name() {
        let ch = make_channel();
        let payload = serde_json::json!({
            "header": { "event_type": "im.message.receive_v1", "event_id": "ev4" },
            "event": {
                "sender": { "sender_id": { "open_id": "ou_user" } },
                "message": {
                    "message_type": "file",
                    "content": "{\"file_key\":\"file_v3_noname\"}",
                    "chat_id": "oc_chat1",
                    "chat_type": "p2p",
                    "message_id": "om_msg4",
                    "create_time": "4000"
                }
            }
        });
        let msgs = ch.parse_event_payload(&payload);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "[DOCUMENT:lark_file_key:file_v3_noname:file_v3_noname]");
    }
    #[test]
    fn lark_download_file_url_matches_region() {
        let ch = make_channel();
        assert_eq!(
            ch.download_file_url("om_msg", "file_key_abc", "file"),
            "https://open.larksuite.com/open-apis/im/v1/messages/om_msg/resources/file_key_abc?type=file"
        );
    }
    #[test]
    fn cardkit_url_lark_platform() {
        let ch = make_channel();
        assert_eq!(
            ch.cardkit_url(),
            "https://open.larksuite.com/open-apis/cardkit/v1/cards"
        );
    }
    #[test]
    fn cardkit_url_feishu_platform() {
        let ch = LarkChannel::new_with_platform(
            "cli_test_app_id".into(),
            "test_app_secret".into(),
            "test_verification_token".into(),
            None,
            vec![],
            LarkPlatform::Feishu,
        );
        assert_eq!(
            ch.cardkit_url(),
            "https://open.feishu.cn/open-apis/cardkit/v1/cards"
        );
    }
    #[test]
    fn card_sequence_increments_per_card_id() {
        let ch = make_channel();
        // Initialize two cards
        ch.card_sequence
            .lock()
            .unwrap()
            .insert("card_a".to_string(), 1);
        ch.card_sequence
            .lock()
            .unwrap()
            .insert("card_b".to_string(), 5);
        // Increment card_a
        {
            let mut seqs = ch.card_sequence.lock().unwrap();
            let seq = seqs.entry("card_a".to_string()).or_insert(1);
            *seq += 1;
            assert_eq!(*seq, 2);
        }
        // Increment card_b
        {
            let mut seqs = ch.card_sequence.lock().unwrap();
            let seq = seqs.entry("card_b".to_string()).or_insert(1);
            *seq += 1;
            assert_eq!(*seq, 6);
        }
        // card_a should still be 2
        assert_eq!(*ch.card_sequence.lock().unwrap().get("card_a").unwrap(), 2);
    }
    #[test]
    fn throttle_enforcement_respects_interval() {
        let ch = make_channel()
            .with_streaming(StreamMode::Partial, 500);
        // Insert a recent timestamp
        ch.last_draft_update
            .lock()
            .unwrap()
            .insert("card_x".to_string(), Instant::now());
        // Should be throttled (elapsed < 500ms)
        let last_updates = ch.last_draft_update.lock().unwrap();
        let last_time = last_updates.get("card_x").unwrap();
        let elapsed = u64::try_from(last_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        assert!(elapsed < 500, "should be within throttle window");
    }
    #[test]
    fn supports_draft_updates_respects_stream_mode() {
        let ch_off = make_channel();
        assert!(!ch_off.supports_draft_updates());
        let ch_on = make_channel()
            .with_streaming(StreamMode::Partial, 500);
        assert!(ch_on.supports_draft_updates());
    }
    #[test]
    fn typing_card_ids_lifecycle() {
        let ch = make_channel();
        // Initially empty
        assert!(ch.typing_card_ids.lock().unwrap().is_empty());
        // Simulate start_typing storing a card_id
        ch.typing_card_ids
            .lock()
            .unwrap()
            .insert("oc_chat_abc".to_string(), "card_typing_1".to_string());
        assert_eq!(
            ch.typing_card_ids.lock().unwrap().get("oc_chat_abc").unwrap(),
            "card_typing_1"
        );
        // Simulate stop_typing removing the entry
        let removed = ch.typing_card_ids.lock().unwrap().remove("oc_chat_abc");
        assert_eq!(removed, Some("card_typing_1".to_string()));
        assert!(ch.typing_card_ids.lock().unwrap().is_empty());
    }
}