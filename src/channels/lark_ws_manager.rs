//! Shared Feishu/Lark WebSocket connection manager.
//!
//! Owns the single WS long-connection per app and broadcasts decoded events
//! to all subscribers (lark channel, docs_sync, future consumers).

use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const FEISHU_WS_BASE_URL: &str = "https://open.feishu.cn";
const LARK_WS_BASE_URL: &str = "https://open.larksuite.com";
const WS_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(300);

// ─────────────────────────────────────────────────────────────────────────────
// Feishu WebSocket pbbp2.proto frame codec (pub(crate) for shared use)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// Feishu WS frame (pbbp2.proto).
/// method=0 → CONTROL (ping/pong)  method=1 → DATA (events)
#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct PbFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PbHeader>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
}

impl PbFrame {
    pub(crate) fn header_value<'a>(&'a self, key: &str) -> &'a str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }
}

/// Server-sent client config (parsed from pong payload).
#[derive(Debug, serde::Deserialize, Default, Clone)]
pub(crate) struct WsClientConfig {
    #[serde(rename = "PingInterval")]
    pub ping_interval: Option<u64>,
}

/// POST /callback/ws/endpoint response.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct WsEndpointResp {
    pub code: i32,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: Option<WsEndpoint>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct WsEndpoint {
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(rename = "ClientConfig")]
    pub client_config: Option<WsClientConfig>,
}

/// Returns true when the WebSocket frame indicates live traffic that should
/// refresh the heartbeat watchdog.
pub(crate) fn should_refresh_last_recv(msg: &WsMsg) -> bool {
    matches!(msg, WsMsg::Binary(_) | WsMsg::Ping(_) | WsMsg::Pong(_))
}

// ─────────────────────────────────────────────────────────────────────────────
// Broadcast event type
// ─────────────────────────────────────────────────────────────────────────────

/// Decoded event broadcast to all subscribers.
#[derive(Debug, Clone)]
pub struct LarkWsEvent {
    pub event_type: String,
    pub payload: Vec<u8>,
}

/// Envelope used to extract `event_type` from the event JSON header.
#[derive(Debug, serde::Deserialize)]
struct EventEnvelope {
    header: EventEnvelopeHeader,
}

#[derive(Debug, serde::Deserialize)]
struct EventEnvelopeHeader {
    event_type: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// LarkWsManager
// ─────────────────────────────────────────────────────────────────────────────

/// Shared WebSocket connection manager for Feishu/Lark.
///
/// Owns the single WS long-connection per app and broadcasts decoded events
/// to all subscribers via `tokio::sync::broadcast`.
pub struct LarkWsManager {
    app_id: String,
    app_secret: String,
    ws_base_url: String,
    locale_header: String,
    event_tx: broadcast::Sender<LarkWsEvent>,
    http: reqwest::Client,
}

impl LarkWsManager {
    /// Create a new manager.
    ///
    /// * `is_feishu` — `true` for Feishu (China), `false` for Lark (international).
    /// * `capacity`  — broadcast channel capacity (default 256).
    pub fn new(
        app_id: String,
        app_secret: String,
        is_feishu: bool,
        capacity: usize,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(capacity);
        let ws_base_url = if is_feishu {
            FEISHU_WS_BASE_URL
        } else {
            LARK_WS_BASE_URL
        }
        .to_string();
        let locale_header = if is_feishu { "zh_cn" } else { "en_us" }.to_string();
        let http = crate::config::schema::build_runtime_proxy_client("lark");
        Self {
            app_id,
            app_secret,
            ws_base_url,
            locale_header,
            event_tx,
            http,
        }
    }

    /// Subscribe to decoded events from the WS connection.
    pub fn subscribe(&self) -> broadcast::Receiver<LarkWsEvent> {
        self.event_tx.subscribe()
    }

    /// Run the manager forever — reconnects automatically on error.
    pub async fn run(&self) -> ! {
        loop {
            if let Err(e) = self.run_once().await {
                tracing::warn!("LarkWsManager: connection error: {e}, reconnecting in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }

    /// Fetch the WSS endpoint URL and client config from Feishu/Lark API.
    async fn get_ws_endpoint(&self) -> anyhow::Result<(String, WsClientConfig)> {
        let resp = self
            .http
            .post(format!("{}/callback/ws/endpoint", self.ws_base_url))
            .header("locale", &self.locale_header)
            .json(&serde_json::json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .await?
            .json::<WsEndpointResp>()
            .await?;
        if resp.code != 0 {
            anyhow::bail!(
                "Lark WS endpoint failed: code={} msg={}",
                resp.code,
                resp.msg.as_deref().unwrap_or("(none)")
            );
        }
        let ep = resp
            .data
            .ok_or_else(|| anyhow::anyhow!("Lark WS endpoint: empty data"))?;
        Ok((ep.url, ep.client_config.unwrap_or_default()))
    }
    /// Single connection lifecycle: connect, heartbeat, read, broadcast.
    async fn run_once(&self) -> anyhow::Result<()> {
        let (wss_url, client_config) = self.get_ws_endpoint().await?;
        let service_id = wss_url
            .split('?')
            .nth(1)
            .and_then(|qs| {
                qs.split('&')
                    .find(|kv| kv.starts_with("service_id="))
                    .and_then(|kv| kv.split('=').nth(1))
                    .and_then(|v| v.parse::<i32>().ok())
            })
            .unwrap_or(0);
        tracing::info!("LarkWsManager: connecting to {wss_url}");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&wss_url).await?;
        let (mut write, mut read) = ws_stream.split();
        tracing::info!("LarkWsManager: WS connected (service_id={service_id})");

        let mut ping_secs = client_config.ping_interval.unwrap_or(30).max(10);
        let mut hb_interval = tokio::time::interval(Duration::from_secs(ping_secs));
        let mut timeout_backoff = Duration::from_millis(500);
        const TIMEOUT_BACKOFF_MAX: Duration = Duration::from_secs(10);
        hb_interval.tick().await; // consume immediate tick

        let mut seq: u64 = 0;
        let mut last_recv = Instant::now();

        // Send initial ping immediately (like the official SDK) so the server
        // starts responding with pongs and we can calibrate the ping_interval.
        seq = seq.wrapping_add(1);
        let initial_ping = PbFrame {
            seq_id: seq,
            log_id: 0,
            service: service_id,
            method: 0,
            headers: vec![PbHeader {
                key: "type".into(),
                value: "ping".into(),
            }],
            payload: None,
        };
        if write
            .send(WsMsg::Binary(initial_ping.encode_to_vec().into()))
            .await
            .is_err()
        {
            anyhow::bail!("LarkWsManager: initial ping failed");
        }
        // message_id → (fragment_slots, created_at) for multi-part reassembly
        type FragEntry = (Vec<Option<Vec<u8>>>, Instant);
        let mut frag_cache: HashMap<String, FragEntry> = HashMap::new();

        loop {
            tokio::select! {
                biased;

                _ = hb_interval.tick() => {
                    seq = seq.wrapping_add(1);
                    let ping = PbFrame {
                        seq_id: seq, log_id: 0, service: service_id, method: 0,
                        headers: vec![PbHeader { key: "type".into(), value: "ping".into() }],
                        payload: None,
                    };
                    if write.send(WsMsg::Binary(ping.encode_to_vec().into())).await.is_err() {
                        tracing::warn!("LarkWsManager: ping failed, reconnecting");
                        break;
                    }
                    // GC stale fragments > 5 min
                    let cutoff = Instant::now().checked_sub(Duration::from_secs(300)).unwrap_or(Instant::now());
                    frag_cache.retain(|_, (_, ts)| *ts > cutoff);
                }
                _ = tokio::time::sleep(timeout_backoff) => {
                    if last_recv.elapsed() > WS_HEARTBEAT_TIMEOUT {
                        tracing::warn!("LarkWsManager: heartbeat timeout, reconnecting");
                        break;
                    }
                    // Exponential backoff: double until cap
                    timeout_backoff = (timeout_backoff * 2).min(TIMEOUT_BACKOFF_MAX);
                }
                msg = read.next() => {
                    let raw = match msg {
                        Some(Ok(ws_msg)) => {
                            if should_refresh_last_recv(&ws_msg) {
                                last_recv = Instant::now();
                                timeout_backoff = Duration::from_millis(500);
                            }
                            match ws_msg {
                                WsMsg::Binary(b) => b,
                                WsMsg::Ping(d) => { let _ = write.send(WsMsg::Pong(d)).await; continue; }
                                WsMsg::Close(_) => { tracing::info!("LarkWsManager: WS closed — reconnecting"); break; }
                                _ => continue,
                            }
                        }
                        None => { tracing::info!("LarkWsManager: WS closed — reconnecting"); break; }
                        Some(Err(e)) => { tracing::error!("LarkWsManager: WS read error: {e}"); break; }
                    };
                    let frame = match PbFrame::decode(&raw[..]) {
                        Ok(f) => f,
                        Err(e) => { tracing::error!("LarkWsManager: proto decode: {e}"); continue; }
                    };
                    // CONTROL frame
                    if frame.method == 0 {
                        if frame.header_value("type") == "pong" {
                            if let Some(p) = &frame.payload {
                                if let Ok(cfg) = serde_json::from_slice::<WsClientConfig>(p) {
                                    if let Some(secs) = cfg.ping_interval {
                                        let secs = secs.max(10);
                                        if secs != ping_secs {
                                            ping_secs = secs;
                                            hb_interval = tokio::time::interval(Duration::from_secs(ping_secs));
                                            tracing::info!("LarkWsManager: ping_interval → {ping_secs}s");
                                        }
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    // DATA frame
                    let msg_id  = frame.header_value("message_id").to_string();
                    let sum     = frame.header_value("sum").parse::<usize>().unwrap_or(1);
                    let seq_num = frame.header_value("seq").parse::<usize>().unwrap_or(0);
                    // ACK immediately (Feishu requires within 3 s)
                    {
                        let mut ack = frame.clone();
                        ack.payload = Some(br#"{"code":200,"headers":{},"data":[]}"#.to_vec());
                        ack.headers.push(PbHeader { key: "biz_rt".into(), value: "0".into() });
                        let _ = write.send(WsMsg::Binary(ack.encode_to_vec().into())).await;
                    }
                    // Fragment reassembly
                    let sum = if sum == 0 { 1 } else { sum };
                    let payload: Vec<u8> = if sum == 1 || msg_id.is_empty() || seq_num >= sum {
                        // Single frame or seq_num >= sum: treat as complete (matches upstream behavior)
                        frame.payload.clone().unwrap_or_default()
                    } else {
                        let entry = frag_cache.entry(msg_id.clone())
                            .or_insert_with(|| (vec![None; sum], Instant::now()));
                        if entry.0.len() != sum { *entry = (vec![None; sum], Instant::now()); }
                        entry.0[seq_num] = frame.payload.clone();
                        if entry.0.iter().all(|s| s.is_some()) {
                            let full: Vec<u8> = entry.0.iter()
                                .flat_map(|s| s.as_deref().unwrap_or(&[]))
                                .copied().collect();
                            frag_cache.remove(&msg_id);
                            full
                        } else { continue; }
                    };
                    // Decode event envelope to extract event_type, then broadcast
                    let event_type = match serde_json::from_slice::<EventEnvelope>(&payload) {
                        Ok(env) => env.header.event_type,
                        Err(_) => String::new(),
                    };
                    let _ = self.event_tx.send(LarkWsEvent { event_type, payload });
                }
            }
        }
        Ok(())
    }
}
