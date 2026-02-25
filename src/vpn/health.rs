//! Health checker and latency tester for VPN proxy nodes.
//!
//! Probes proxy nodes by sending HTTP requests through the SOCKS5 proxy
//! to a connectivity check endpoint. Supports parallel health checks
//! and a background monitoring loop with graceful shutdown.

use std::time::{Duration, Instant};

use anyhow::Result;
use tokio_util::sync::CancellationToken;

/// Connectivity check URL — returns HTTP 204 on success.
const PROBE_URL: &str = "http://connectivitycheck.gstatic.com/generate_204";

/// Probe timeout per node.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Connect timeout for the probe client.
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(4);

/// Default background health check interval.
const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Health status of a proxy node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Node responded with expected status within timeout.
    Healthy,
    /// Node failed to respond or returned unexpected status.
    Unhealthy,
    /// Node has not been checked yet.
    Unknown,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Unhealthy => write!(f, "unhealthy"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Result of a single health check probe.
#[derive(Debug, Clone)]
pub struct HealthResult {
    /// Current health status.
    pub status: NodeStatus,
    /// Round-trip latency in milliseconds (present only when healthy).
    pub latency_ms: Option<u64>,
    /// When this check was performed.
    pub checked_at: Instant,
}

impl HealthResult {
    /// Create a healthy result with measured latency.
    fn healthy(latency: Duration) -> Self {
        Self {
            status: NodeStatus::Healthy,
            latency_ms: Some(latency.as_millis() as u64),
            checked_at: Instant::now(),
        }
    }

    /// Create an unhealthy result.
    fn unhealthy() -> Self {
        Self {
            status: NodeStatus::Unhealthy,
            latency_ms: None,
            checked_at: Instant::now(),
        }
    }

    /// Create an unknown (not-yet-checked) result.
    pub fn unknown() -> Self {
        Self {
            status: NodeStatus::Unknown,
            latency_ms: None,
            checked_at: Instant::now(),
        }
    }
}

/// Health checker for VPN proxy nodes.
pub struct HealthChecker;

impl HealthChecker {
    /// Probe a single proxy node and return its health status.
    pub async fn check_node(proxy_url: &str) -> HealthResult {
        match Self::measure_latency(proxy_url).await {
            Ok(latency) => HealthResult::healthy(latency),
            Err(_) => HealthResult::unhealthy(),
        }
    }

    /// Measure round-trip latency through a SOCKS5 proxy.
    pub async fn measure_latency(proxy_url: &str) -> Result<Duration> {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| anyhow::anyhow!("invalid proxy URL '{proxy_url}': {e}"))?;

        let client = reqwest::Client::builder()
            .proxy(proxy)
            .timeout(PROBE_TIMEOUT)
            .connect_timeout(PROBE_CONNECT_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build probe client: {e}"))?;

        let start = Instant::now();
        let resp = client
            .get(PROBE_URL)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("probe failed through {proxy_url}: {e}"))?;

        let status = resp.status().as_u16();
        if status == 204 || status == 200 {
            Ok(start.elapsed())
        } else {
            anyhow::bail!("unexpected probe status {status} through {proxy_url}");
        }
    }

    /// Check all nodes in parallel. Each entry is `(name, proxy_url)`.
    /// Returns `(name, HealthResult)` for every node.
    pub async fn check_all(nodes: &[(String, String)]) -> Vec<(String, HealthResult)> {
        let futures: Vec<_> = nodes
            .iter()
            .map(|(name, url)| {
                let name = name.clone();
                let url = url.clone();
                async move {
                    let result = Self::check_node(&url).await;
                    (name, result)
                }
            })
            .collect();

        futures_util::future::join_all(futures).await
    }
    /// Spawn a background health-check loop.
    ///
    /// Runs `check_all` every `interval` (default 30s). The returned
    /// `CancellationToken` can be cancelled to stop the loop gracefully.
    /// The callback `on_results` is invoked after each round.
    pub fn spawn_background_loop<F>(
        nodes: Vec<(String, String)>,
        interval: Option<Duration>,
        token: CancellationToken,
        on_results: F,
    ) -> tokio::task::JoinHandle<()>
    where
        F: Fn(Vec<(String, HealthResult)>) + Send + Sync + 'static,
    {
        let interval = interval.unwrap_or(DEFAULT_CHECK_INTERVAL);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // First tick fires immediately.
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        tracing::debug!("health check loop cancelled");
                        break;
                    }
                    _ = ticker.tick() => {
                        let results = Self::check_all(&nodes).await;
                        on_results(results);
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_result_healthy_construction() {
        let result = HealthResult::healthy(Duration::from_millis(42));
        assert_eq!(result.status, NodeStatus::Healthy);
        assert_eq!(result.latency_ms, Some(42));
    }

    #[test]
    fn health_result_unhealthy_construction() {
        let result = HealthResult::unhealthy();
        assert_eq!(result.status, NodeStatus::Unhealthy);
        assert_eq!(result.latency_ms, None);
    }

    #[test]
    fn health_result_unknown_construction() {
        let result = HealthResult::unknown();
        assert_eq!(result.status, NodeStatus::Unknown);
        assert_eq!(result.latency_ms, None);
    }

    #[test]
    fn node_status_display() {
        assert_eq!(NodeStatus::Healthy.to_string(), "healthy");
        assert_eq!(NodeStatus::Unhealthy.to_string(), "unhealthy");
        assert_eq!(NodeStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn node_status_equality() {
        assert_eq!(NodeStatus::Healthy, NodeStatus::Healthy);
        assert_ne!(NodeStatus::Healthy, NodeStatus::Unhealthy);
        assert_ne!(NodeStatus::Unknown, NodeStatus::Healthy);
    }
    #[tokio::test]
    async fn background_loop_starts_and_cancels() {
        let token = CancellationToken::new();
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = call_count.clone();
        let handle = HealthChecker::spawn_background_loop(
            vec![],
            Some(Duration::from_millis(50)),
            token.clone(),
            move |_results| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
        token.cancel();
        handle.await.unwrap();
        let count = call_count.load(std::sync::atomic::Ordering::Relaxed);
        assert!(count >= 1, "callback should have been invoked at least once");
    }
}
