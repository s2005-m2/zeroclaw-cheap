//! Node persistence (disk cache) and selection/failover for VPN proxy nodes.
//!
//! Provides async save/load of `ProxyNode` lists to a JSON file on disk,
//! with a last-fetched timestamp. Corrupt files are handled gracefully
//! (logged as warning, returns `None`).

use super::health::{HealthResult, NodeStatus};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::subscription::ProxyNode;

/// Wrapper stored on disk: nodes + fetch timestamp.
#[derive(Debug, Serialize, Deserialize)]
pub struct CachedNodes {
    /// ISO-8601 timestamp of when nodes were last fetched.
    pub fetched_at: String,
    /// The cached proxy nodes.
    pub nodes: Vec<ProxyNode>,
}


/// Disk cache for proxy nodes.
///
/// Persistence layer only — no selection or health-check logic.
pub struct NodeCache;

impl NodeCache {
    /// Save nodes to a JSON file at `path`.
    ///
    /// Creates parent directories if they don't exist.
    pub async fn save(nodes: &[ProxyNode], path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let cached = CachedNodes {
            fetched_at: chrono::Utc::now().to_rfc3339(),
            nodes: nodes.to_vec(),
        };

        let json = serde_json::to_string_pretty(&cached)?;
        tokio::fs::write(path, json).await?;
        Ok(())
    }

    /// Load cached nodes from a JSON file at `path`.
    ///
    /// Returns `Ok(None)` if the file doesn't exist or contains corrupt JSON.
    /// Corrupt files are logged as warnings, not treated as hard errors.
    pub async fn load(path: &Path) -> Result<Option<Vec<ProxyNode>>> {
        if !path.exists() {
            return Ok(None);
        }

        let data = match tokio::fs::read_to_string(path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("failed to read node cache at {}: {e}", path.display());
                return Ok(None);
            }
        };

        match serde_json::from_str::<CachedNodes>(&data) {
            Ok(cached) => Ok(Some(cached.nodes)),
            Err(e) => {
                tracing::warn!(
                    "corrupt node cache at {}, ignoring: {e}",
                    path.display()
                );
                Ok(None)
            }
        }
    }

    /// Default cache path: `~/.zeroclaw/state/vpn/nodes.json`.
    pub fn default_cache_path() -> PathBuf {
        let base = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        base.join(".zeroclaw").join("state").join("vpn").join("nodes.json")
    }
}

/// Node manager with selection strategy and failover logic.
///
/// Tracks a pool of proxy nodes and an optional active node.
/// Selection picks the lowest-latency healthy node; failover
/// skips the current active and picks the next best.
pub struct NodeManager {
    nodes: Vec<ProxyNode>,
    active: Option<String>,
}

impl NodeManager {
    /// Create a new manager from a list of proxy nodes.
    pub fn new(nodes: Vec<ProxyNode>) -> Self {
        Self {
            nodes,
            active: None,
        }
    }

    /// Select the best (lowest-latency healthy) node.
    pub fn select_best_node(&self, health_results: &[(String, HealthResult)]) -> Option<&ProxyNode> {
        let mut healthy: Vec<(&String, u64)> = health_results
            .iter()
            .filter(|(_, hr)| hr.status == NodeStatus::Healthy)
            .filter_map(|(name, hr)| hr.latency_ms.map(|ms| (name, ms)))
            .collect();

        healthy.sort_by_key(|(_, ms)| *ms);

        healthy
            .first()
            .and_then(|(name, _)| self.nodes.iter().find(|n| &n.name == *name))
    }

    /// Failover: select next best healthy node, skipping the current active.
    ///
    /// Returns `None` when all nodes are unhealthy.
    pub fn failover(&mut self, health_results: &[(String, HealthResult)]) -> Option<&ProxyNode> {
        let skip = self.active.as_deref();

        let mut healthy: Vec<(&String, u64)> = health_results
            .iter()
            .filter(|(name, _)| skip.map_or(true, |s| s != name))
            .filter(|(_, hr)| hr.status == NodeStatus::Healthy)
            .filter_map(|(name, hr)| hr.latency_ms.map(|ms| (name, ms)))
            .collect();

        healthy.sort_by_key(|(_, ms)| *ms);

        if let Some((name, _)) = healthy.first() {
            self.active = Some(name.to_string());
            self.nodes.iter().find(|n| &n.name == *name)
        } else {
            None
        }
    }

    /// Return the currently active node, if any.
    pub fn active_node(&self) -> Option<&ProxyNode> {
        self.active
            .as_deref()
            .and_then(|name| self.nodes.iter().find(|n| n.name == name))
    }

    /// Set the active node by name. Returns `true` if the node exists.
    pub fn set_active(&mut self, node_name: &str) -> bool {
        if self.nodes.iter().any(|n| n.name == node_name) {
            self.active = Some(node_name.to_string());
            true
        } else {
            false
        }
    }

    /// Return all managed nodes.
    pub fn all_nodes(&self) -> &[ProxyNode] {
        &self.nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpn::subscription::NodeType;
    use super::health::HealthResult;
    use std::time::Duration;

    fn sample_nodes() -> Vec<ProxyNode> {
        vec![
            ProxyNode {
                name: "test-vmess".into(),
                node_type: NodeType::VMess,
                server: "vmess.example.com".into(),
                port: 443,
                raw_config: serde_json::json!({"uuid": "test"}),
            },
            ProxyNode {
                name: "test-trojan".into(),
                node_type: NodeType::Trojan,
                server: "trojan.example.com".into(),
                port: 443,
                raw_config: serde_json::json!({"password": "placeholder"}),
            },
        ]
    }

    #[tokio::test]
    async fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vpn").join("nodes.json");

        let nodes = sample_nodes();
        NodeCache::save(&nodes, &path).await.unwrap();

        let loaded = NodeCache::load(&path).await.unwrap().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "test-vmess");
        assert_eq!(loaded[0].node_type, NodeType::VMess);
        assert_eq!(loaded[0].server, "vmess.example.com");
        assert_eq!(loaded[0].port, 443);
        assert_eq!(loaded[1].name, "test-trojan");
        assert_eq!(loaded[1].node_type, NodeType::Trojan);
    }
    #[tokio::test]
    async fn load_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let result = NodeCache::load(&path).await.unwrap();
        assert!(result.is_none());
    }
    #[tokio::test]
    async fn load_corrupt_json_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.json");
        tokio::fs::write(&path, "not valid json {{{").await.unwrap();
        let result = NodeCache::load(&path).await.unwrap();
        assert!(result.is_none());
    }
    #[tokio::test]
    async fn save_stores_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ts.json");
        NodeCache::save(&sample_nodes(), &path).await.unwrap();
        let raw = tokio::fs::read_to_string(&path).await.unwrap();
        let cached: CachedNodes = serde_json::from_str(&raw).unwrap();
        assert!(!cached.fetched_at.is_empty());
        assert_eq!(cached.nodes.len(), 2);
    }

    #[test]
    fn default_cache_path_ends_with_nodes_json() {
        let path = NodeCache::default_cache_path();
        assert!(path.ends_with("state/vpn/nodes.json") || path.ends_with("state\\vpn\\nodes.json"));
    }

    // --- NodeManager tests ---

    fn three_nodes() -> Vec<ProxyNode> {
        vec![
            ProxyNode {
                name: "node-a".into(),
                node_type: NodeType::VMess,
                server: "a.example.com".into(),
                port: 443,
                raw_config: serde_json::json!({}),
            },
            ProxyNode {
                name: "node-b".into(),
                node_type: NodeType::Trojan,
                server: "b.example.com".into(),
                port: 443,
                raw_config: serde_json::json!({}),
            },
            ProxyNode {
                name: "node-c".into(),
                node_type: NodeType::Shadowsocks,
                server: "c.example.com".into(),
                port: 8388,
                raw_config: serde_json::json!({}),
            },
        ]
    }

    fn make_health(name: &str, status: NodeStatus, latency: Option<u64>) -> (String, HealthResult) {
        (
            name.to_string(),
            HealthResult {
                status,
                latency_ms: latency,
                checked_at: std::time::Instant::now(),
            },
        )
    }
    #[test]
    fn select_best_node_picks_lowest_latency() {
        let mgr = NodeManager::new(three_nodes());
        let health = vec![
            make_health("node-a", NodeStatus::Healthy, Some(100)),
            make_health("node-b", NodeStatus::Healthy, Some(50)),
            make_health("node-c", NodeStatus::Healthy, Some(200)),
        ];
        let best = mgr.select_best_node(&health).unwrap();
        assert_eq!(best.name, "node-b");
    }

    #[test]
    fn select_best_node_skips_unhealthy() {
        let mgr = NodeManager::new(three_nodes());
        let health = vec![
            make_health("node-a", NodeStatus::Unhealthy, None),
            make_health("node-b", NodeStatus::Healthy, Some(80)),
            make_health("node-c", NodeStatus::Unhealthy, None),
        ];
        let best = mgr.select_best_node(&health).unwrap();
        assert_eq!(best.name, "node-b");
    }

    #[test]
    fn select_best_node_all_unhealthy_returns_none() {
        let mgr = NodeManager::new(three_nodes());
        let health = vec![
            make_health("node-a", NodeStatus::Unhealthy, None),
            make_health("node-b", NodeStatus::Unhealthy, None),
            make_health("node-c", NodeStatus::Unknown, None),
        ];
        assert!(mgr.select_best_node(&health).is_none());
    }
    #[test]
    fn failover_skips_active_node() {
        let mut mgr = NodeManager::new(three_nodes());
        mgr.set_active("node-b");
        let health = vec![
            make_health("node-a", NodeStatus::Healthy, Some(100)),
            make_health("node-b", NodeStatus::Healthy, Some(50)),
            make_health("node-c", NodeStatus::Healthy, Some(80)),
        ];
        let next = mgr.failover(&health).unwrap();
        // node-b is skipped (active), node-c (80ms) beats node-a (100ms)
        assert_eq!(next.name, "node-c");
        assert_eq!(mgr.active_node().unwrap().name, "node-c");
    }
    #[test]
    fn failover_all_unhealthy_returns_none() {
        let mut mgr = NodeManager::new(three_nodes());
        mgr.set_active("node-a");
        let health = vec![
            make_health("node-a", NodeStatus::Healthy, Some(10)),
            make_health("node-b", NodeStatus::Unhealthy, None),
            make_health("node-c", NodeStatus::Unhealthy, None),
        ];
        // node-a is skipped (active), rest unhealthy
        assert!(mgr.failover(&health).is_none());
    }
    #[test]
    fn set_active_existing_node() {
        let mut mgr = NodeManager::new(three_nodes());
        assert!(mgr.active_node().is_none());
        assert!(mgr.set_active("node-c"));
        assert_eq!(mgr.active_node().unwrap().name, "node-c");
    }
    #[test]
    fn set_active_nonexistent_returns_false() {
        let mut mgr = NodeManager::new(three_nodes());
        assert!(!mgr.set_active("no-such-node"));
        assert!(mgr.active_node().is_none());
    }
    #[test]
    fn all_nodes_returns_full_list() {
        let mgr = NodeManager::new(three_nodes());
        assert_eq!(mgr.all_nodes().len(), 3);
    }

}
