//! Feishu WebSocket event subscriber for `drive.file.edit_v1`.
//!
//! Connects to the Feishu long-connection endpoint and listens for document
//! edit events. When the synced document is edited, emits a notification on
//! a tokio channel so the sync engine can pull the latest content.
//!
//! Reuses the same pbbp2.proto frame codec as `src/channels/lark.rs`.

use anyhow::{bail, Result};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as WsMsg;

const FEISHU_WS_BASE_URL: &str = "https://open.feishu.cn";

/// Heartbeat timeout — reconnect if no binary frame within this window.
const WS_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(300);

// ── pbbp2.proto frame codec (duplicated from lark.rs per rule-of-three) ──

#[derive(Clone, PartialEq, prost::Message)]
struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PbFrame {
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
    fn header_value<'a>(&'a self, key: &str) -> &'a str {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
            .unwrap_or("")
    }
}

#[derive(Debug, serde::Deserialize, Default, Clone)]
struct WsClientConfig {
    #[serde(rename = "PingInterval")]
    ping_interval: Option<u64>,
}

#[derive(Debug, serde::Deserialize)]
struct WsEndpointResp {
    code: i32,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<WsEndpoint>,
}

#[derive(Debug, serde::Deserialize)]
struct WsEndpoint {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "ClientConfig")]
    client_config: Option<WsClientConfig>,
}

/// Feishu drive event envelope.
#[derive(Debug, serde::Deserialize)]
struct DriveEvent {
    header: DriveEventHeader,
    #[allow(dead_code)]
    event: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
struct DriveEventHeader {
    event_type: String,
    #[allow(dead_code)]
    event_id: String,
}

/// Feishu WebSocket event subscriber.
///
/// Connects to the Feishu long-connection endpoint, listens for
/// `drive.file.edit_v1` events, and sends a notification on `tx`
/// whenever the watched document is edited.
pub struct EventSubscriber {
    app_id: String,
    app_secret: String,
    document_id: String,
    http: reqwest::Client,
}

impl EventSubscriber {
    /// Create a new event subscriber.
    pub fn new(app_id: String, app_secret: String, document_id: String) -> Self {
        let http = crate::config::schema::build_runtime_proxy_client("channel.feishu");
        Self {
            app_id,
            app_secret,
            document_id,
            http,
        }
    }

    /// Acquire a WebSocket endpoint URL from Feishu.
    async fn get_ws_endpoint(&self) -> Result<(String, WsClientConfig)> {
        let resp = self
            .http
            .post(format!("{FEISHU_WS_BASE_URL}/callback/ws/endpoint"))
            .header("locale", "zh_cn")
            .json(&serde_json::json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .await?
            .json::<WsEndpointResp>()
            .await?;
        if resp.code != 0 {
            bail!(
                "docs_sync WS endpoint failed: code={} msg={}",
                resp.code,
                resp.msg.as_deref().unwrap_or("(none)")
            );
        }
        let ep = resp
            .data
            .ok_or_else(|| anyhow::anyhow!("docs_sync WS endpoint: empty data"))?;
        Ok((ep.url, ep.client_config.unwrap_or_default()))
    }
}

impl EventSubscriber {
    /// Run the event subscriber loop with automatic reconnection.
    ///
    /// Sends `()` on `tx` whenever a `drive.file.edit_v1` event fires
    /// for the configured document. The caller should trigger a
    /// remote-to-local sync on each receive.
    pub async fn run(&self, tx: mpsc::Sender<()>) {
        loop {
            match self.listen_ws(&tx).await {
                Ok(()) => {
                    tracing::info!("docs_sync: WS closed, reconnecting in 5s");
                }
                Err(e) => {
                    tracing::warn!("docs_sync: WS error: {e}, reconnecting in 5s");
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
    /// Single WebSocket session. Returns `Ok(())` when the connection closes.
    async fn listen_ws(&self, tx: &mpsc::Sender<()>) -> Result<()> {
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
        tracing::info!("docs_sync: connecting to WS endpoint");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&wss_url).await?;
        let (mut write, mut read) = ws_stream.split();
        tracing::info!("docs_sync: WS connected (service_id={service_id})");

        let mut ping_secs = client_config.ping_interval.unwrap_or(120).max(10);
        let mut hb_interval = tokio::time::interval(Duration::from_secs(ping_secs));
        let mut timeout_check = tokio::time::interval(Duration::from_secs(10));
        hb_interval.tick().await;

        let mut seq: u64 = 0;
        let mut last_recv = Instant::now();

        // Send initial ping
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
            bail!("docs_sync: initial ping failed");
        }
        // Fragment reassembly cache: message_id → (slots, created_at)
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
                        tracing::warn!("docs_sync: ping failed, reconnecting");
                        break;
                    }
                    // GC stale fragments > 5 min
                    let cutoff = Instant::now().checked_sub(Duration::from_secs(300)).unwrap_or(Instant::now());
                    frag_cache.retain(|_, (_, ts)| *ts > cutoff);
                }
                _ = timeout_check.tick() => {
                    if last_recv.elapsed() > WS_HEARTBEAT_TIMEOUT {
                        tracing::warn!("docs_sync: heartbeat timeout, reconnecting");
                        break;
                    }
                }
                msg = read.next() => {
                    let raw = match msg {
                        Some(Ok(ws_msg)) => {
                            if matches!(ws_msg, WsMsg::Binary(_) | WsMsg::Ping(_) | WsMsg::Pong(_)) {
                                last_recv = Instant::now();
                            }
                            match ws_msg {
                                WsMsg::Binary(b) => b,
                                WsMsg::Ping(d) => { let _ = write.send(WsMsg::Pong(d)).await; continue; }
                                WsMsg::Close(_) => break,
                                _ => continue,
                            }
                        }
                        None => break,
                        Some(Err(e)) => { tracing::error!("docs_sync: WS read error: {e}"); break; }
                    };
                    let frame = match PbFrame::decode(&raw[..]) {
                        Ok(f) => f,
                        Err(e) => { tracing::error!("docs_sync: proto decode: {e}"); continue; }
                    };
                    // CONTROL frame (pong)
                    if frame.method == 0 {
                        if frame.header_value("type") == "pong" {
                            if let Some(p) = &frame.payload {
                                if let Ok(cfg) = serde_json::from_slice::<WsClientConfig>(p) {
                                    if let Some(secs) = cfg.ping_interval {
                                        let secs = secs.max(10);
                                        if secs != ping_secs {
                                            ping_secs = secs;
                                            hb_interval = tokio::time::interval(Duration::from_secs(ping_secs));
                                        }
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    // DATA frame
                    let msg_type = frame.header_value("type").to_string();
                    let msg_id = frame.header_value("message_id").to_string();
                    let sum = frame.header_value("sum").parse::<usize>().unwrap_or(1).max(1);
                    let seq_num = frame.header_value("seq").parse::<usize>().unwrap_or(0);
                    // ACK immediately (Feishu requires within 3s)
                    {
                        let mut ack = frame.clone();
                        ack.payload = Some(br#"{"code":200,"headers":{},"data":[]}"#.to_vec());
                        ack.headers.push(PbHeader { key: "biz_rt".into(), value: "0".into() });
                        let _ = write.send(WsMsg::Binary(ack.encode_to_vec().into())).await;
                    }
                    // Fragment reassembly
                    let payload: Vec<u8> = if sum == 1 || msg_id.is_empty() {
                        frame.payload.clone().unwrap_or_default()
                    } else if seq_num >= sum {
                        continue;
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
                    if msg_type != "event" { continue; }
                    let event: DriveEvent = match serde_json::from_slice(&payload) {
                        Ok(e) => e,
                        Err(e) => { tracing::error!("docs_sync: event JSON: {e}"); continue; }
                    };
                    if event.header.event_type != "drive.file.edit_v1" {
                        tracing::debug!(
                            "docs_sync: ignoring event type={}",
                            event.header.event_type
                        );
                        continue;
                    }
                    // Optionally filter by document_id if the event payload contains it.
                    // Feishu drive.file.edit_v1 event payload has file_token in event.file_token.
                    if !self.document_id.is_empty() {
                        let file_token = event.event
                            .get("file_token")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !file_token.is_empty() && file_token != self.document_id {
                            tracing::debug!(
                                "docs_sync: edit event for different doc {file_token}, skipping"
                            );
                            continue;
                        }
                    }
                    tracing::info!("docs_sync: received drive.file.edit_v1 event");
                    // Notify the sync engine to pull remote changes.
                    let _ = tx.try_send(());
                }
            }
        }
        Ok(())
    }
}
