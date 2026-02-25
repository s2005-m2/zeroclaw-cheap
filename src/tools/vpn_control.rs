//! Agent-facing VPN control tool.
//!
//! Exposes VPN lifecycle management (enable/disable, node switching, status,
//! bypass list mutation) to the agent via the `Tool` trait. All VPN logic is
//! delegated to components in `crate::vpn`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use super::traits::{Tool, ToolResult};
use crate::security::SecurityPolicy;
use crate::vpn::{
    BypassChecker, ClashRuntime, HealthChecker, NodeManager, SubscriptionParser, VpnProxyBridge,
};

// ── Shared VPN state ────────────────────────────────────────────────

/// Aggregated VPN runtime state shared across the tool and background tasks.
///
/// Wrapped in `Arc<RwLock<VpnState>>` so the tool can read status without
/// blocking ongoing health checks or node switches.
pub struct VpnState {
    pub runtime: Option<ClashRuntime>,
    pub node_manager: NodeManager,
    pub bypass_checker: BypassChecker,
    pub bridge: VpnProxyBridge,
    pub health_cancel: Option<tokio_util::sync::CancellationToken>,
    pub last_health: Vec<(String, crate::vpn::HealthResult)>,
    /// Clash subscription URL for fetching proxy nodes.
    pub subscription_url: Option<String>,
    /// SOCKS5 listen port for the Clash runtime.
    pub listen_port: u16,
    /// Background health check interval in seconds.
    pub health_check_interval_secs: u64,
}

// ── VpnControlTool ──────────────────────────────────────────────────

pub struct VpnControlTool {
    security: Arc<SecurityPolicy>,
    state: Arc<RwLock<VpnState>>,
}

impl VpnControlTool {
    pub fn new(security: Arc<SecurityPolicy>, state: Arc<RwLock<VpnState>>) -> Self {
        Self { security, state }
    }

    fn require_write_access(&self) -> Option<ToolResult> {
        if !self.security.can_act() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }
        if !self.security.record_action() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }
        None
    }
}

// ── Action handlers ──────────────────────────────────────────────────

impl VpnControlTool {
    async fn handle_status(&self) -> anyhow::Result<ToolResult> {
        let state = self.state.read().await;
        let enabled = state.runtime.is_some();
        let active_node = state.node_manager.active_node();
        let (node_name, latency_ms, health) = match active_node {
            Some(node) => {
                let hr = state
                    .last_health
                    .iter()
                    .find(|(n, _)| n == &node.name)
                    .map(|(_, h)| h);
                (
                    Some(node.name.as_str()),
                    hr.and_then(|h| h.latency_ms),
                    hr.map(|h| h.status.to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                )
            }
            None => (None, None, "unknown".to_string()),
        };
        let listen_port = state.runtime.as_ref().map(|r| r.socks_port()).unwrap_or(0);
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "enabled": enabled,
                "active_node": node_name,
                "latency_ms": latency_ms,
                "health": health,
                "listen_port": listen_port,
            }))?,
            error: None,
        })
    }
    async fn handle_enable(&self) -> anyhow::Result<ToolResult> {
        let mut state = self.state.write().await;
        if state.runtime.is_some() {
            anyhow::bail!("VPN is already enabled");
        }
        let sub_url = state.subscription_url.as_deref().unwrap_or_default();
        if sub_url.is_empty() {
            anyhow::bail!("No VPN subscription URL configured");
        }
        let nodes = SubscriptionParser::fetch_and_parse(sub_url).await?;
        let listen_port = state.listen_port;
        let config_yaml = crate::vpn::generate_clash_config(&nodes, listen_port)?;
        let runtime = ClashRuntime::start(&config_yaml, listen_port).await?;
        let proxy_url = runtime.local_proxy_url();
        state.bridge.activate(&proxy_url, &state.bypass_checker)?;
        state.node_manager = NodeManager::new(nodes.clone());
        let check_pairs: Vec<(String, String)> = nodes
            .iter()
            .map(|n| (n.name.clone(), proxy_url.clone()))
            .collect();
        let health_results = HealthChecker::check_all(&check_pairs).await;
        if let Some(best) = state.node_manager.select_best_node(&health_results) {
            let best_name = best.name.clone();
            state.node_manager.set_active(&best_name);
        }
        state.last_health = health_results;
        let token = tokio_util::sync::CancellationToken::new();
        let health_state = Arc::clone(&self.state);
        let interval_secs = state.health_check_interval_secs;
        HealthChecker::spawn_background_loop(
            check_pairs,
            Some(std::time::Duration::from_secs(interval_secs)),
            token.clone(),
            move |results| {
                let st = Arc::clone(&health_state);
                tokio::spawn(async move {
                    let mut guard = st.write().await;
                    guard.last_health = results;
                });
            },
        );
        state.health_cancel = Some(token);
        state.runtime = Some(runtime);
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "VPN enabled",
                "listen_port": listen_port,
                "active_node": state.node_manager.active_node().map(|n| &n.name),
                "total_nodes": state.node_manager.all_nodes().len(),
            }))?,
            error: None,
        })
    }
    async fn handle_disable(&self) -> anyhow::Result<ToolResult> {
        let mut state = self.state.write().await;
        if state.runtime.is_none() {
            anyhow::bail!("VPN is not enabled");
        }
        if let Some(token) = state.health_cancel.take() {
            token.cancel();
        }
        state.bridge.deactivate()?;
        if let Some(ref mut rt) = state.runtime {
            rt.stop().await?;
        }
        state.runtime = None;
        state.last_health.clear();
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "VPN disabled"
            }))?,
            error: None,
        })
    }
    async fn handle_list_nodes(&self) -> anyhow::Result<ToolResult> {
        let state = self.state.read().await;
        let nodes: Vec<Value> = state
            .node_manager
            .all_nodes()
            .iter()
            .map(|n| {
                let hr = state
                    .last_health
                    .iter()
                    .find(|(name, _)| name == &n.name)
                    .map(|(_, h)| h);
                json!({
                    "name": n.name,
                    "server": n.server,
                    "port": n.port,
                    "node_type": n.node_type.to_string(),
                    "status": hr.map(|h| h.status.to_string())
                        .unwrap_or_else(|| "unknown".into()),
                    "latency_ms": hr.and_then(|h| h.latency_ms),
                })
            })
            .collect();
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({ "nodes": nodes }))?,
            error: None,
        })
    }
    async fn handle_switch_node(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let node_name = args
            .get("node_name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'node_name'"))?;
        let mut state = self.state.write().await;
        if !state.node_manager.set_active(node_name) {
            anyhow::bail!("Node '{node_name}' not found in node list");
        }
        if let Some(ref mut rt) = state.runtime {
            rt.switch_node(node_name).await?;
        }
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": format!("Switched to node '{}'", node_name),
                "active_node": node_name,
            }))?,
            error: None,
        })
    }
    async fn handle_refresh(&self) -> anyhow::Result<ToolResult> {
        let mut state = self.state.write().await;
        let sub_url = state.subscription_url.as_deref().unwrap_or_default();
        if sub_url.is_empty() {
            anyhow::bail!("No VPN subscription URL configured");
        }
        let nodes = SubscriptionParser::fetch_and_parse(sub_url).await?;
        state.node_manager = NodeManager::new(nodes.clone());
        if let Some(ref rt) = state.runtime {
            let proxy_url = rt.local_proxy_url();
            let check_pairs: Vec<(String, String)> = nodes
                .iter()
                .map(|n| (n.name.clone(), proxy_url.clone()))
                .collect();
            let health_results = HealthChecker::check_all(&check_pairs).await;
            if let Some(best) = state.node_manager.select_best_node(&health_results) {
                let best_name = best.name.clone();
                state.node_manager.set_active(&best_name);
            }
            state.last_health = health_results;
        }
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": "Subscription refreshed",
                "total_nodes": state.node_manager.all_nodes().len(),
                "active_node": state.node_manager.active_node().map(|n| &n.name),
            }))?,
            error: None,
        })
    }
    async fn handle_add_bypass(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let domain = args
            .get("domain")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'domain'"))?;
        let mut state = self.state.write().await;
        state.bypass_checker.add_domain(domain);
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": format!("Added '{}' to bypass list", domain),
            }))?,
            error: None,
        })
    }
    async fn handle_remove_bypass(&self, args: &Value) -> anyhow::Result<ToolResult> {
        let domain = args
            .get("domain")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'domain'"))?;
        let mut state = self.state.write().await;
        state.bypass_checker.remove_domain(domain);
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&json!({
                "message": format!("Removed '{}' from bypass list", domain),
            }))?,
            error: None,
        })
    }
}
// ── Tool trait implementation ────────────────────────────────────────
#[async_trait]
impl Tool for VpnControlTool {
    fn name(&self) -> &str {
        "vpn_control"
    }
    fn description(&self) -> &str {
        "Manage VPN proxy lifecycle: enable/disable, switch nodes, check status, refresh subscription, manage bypass list"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "enable", "disable", "list_nodes", "switch_node", "refresh", "add_bypass", "remove_bypass"],
                    "description": "VPN control action to perform"
                },
                "node_name": {
                    "type": "string",
                    "description": "Node name for switch_node action"
                },
                "domain": {
                    "type": "string",
                    "description": "Domain for add_bypass/remove_bypass actions"
                }
            },
            "required": ["action"]
        })
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let result = match action.as_str() {
            "status" => self.handle_status().await,
            "list_nodes" => self.handle_list_nodes().await,
            "enable" | "disable" | "switch_node" | "refresh"
            | "add_bypass" | "remove_bypass" => {
                if let Some(blocked) = self.require_write_access() {
                    return Ok(blocked);
                }
                match action.as_str() {
                    "enable" => self.handle_enable().await,
                    "disable" => self.handle_disable().await,
                    "switch_node" => self.handle_switch_node(&args).await,
                    "refresh" => self.handle_refresh().await,
                    "add_bypass" => self.handle_add_bypass(&args).await,
                    "remove_bypass" => self.handle_remove_bypass(&args).await,
                    _ => unreachable!("handled above"),
                }
            }
            other => anyhow::bail!(
                "Unknown action '{}'. Valid: status, enable, disable, list_nodes, switch_node, refresh, add_bypass, remove_bypass",
                other
            ),
        };
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(error.to_string()),
            }),
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{AutonomyLevel, SecurityPolicy};

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }
    fn test_state() -> Arc<RwLock<VpnState>> {
        Arc::new(RwLock::new(VpnState {
            runtime: None,
            node_manager: NodeManager::new(vec![]),
            bypass_checker: BypassChecker::new(&[]),
            bridge: VpnProxyBridge::new(),
            health_cancel: None,
            last_health: vec![],
            subscription_url: None,
            listen_port: 7890,
            health_check_interval_secs: 30,
        }))
    }
    fn test_tool() -> VpnControlTool {
        VpnControlTool::new(test_security(), test_state())
    }
    #[test]
    fn vpn_control_name_and_schema() {
        let tool = test_tool();
        assert_eq!(tool.name(), "vpn_control");
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        let action_enum = &schema["properties"]["action"]["enum"];
        assert!(action_enum.is_array());
        let actions: Vec<&str> = action_enum
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(actions.contains(&"status"));
        assert!(actions.contains(&"enable"));
        assert!(actions.contains(&"disable"));
        assert!(actions.contains(&"list_nodes"));
        assert!(actions.contains(&"switch_node"));
        assert!(actions.contains(&"refresh"));
        assert!(actions.contains(&"add_bypass"));
        assert!(actions.contains(&"remove_bypass"));
        assert_eq!(schema["required"][0], "action");
    }
    #[tokio::test]
    async fn vpn_control_invalid_action() {
        let tool = test_tool();
        let result = tool.execute(json!({"action": "bogus"})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("Unknown action"));
    }
    #[tokio::test]
    async fn vpn_control_status_returns_json() {
        let tool = test_tool();
        let result = tool.execute(json!({"action": "status"})).await.unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed.get("enabled").is_some());
        assert!(parsed.get("active_node").is_some());
        assert!(parsed.get("latency_ms").is_some());
        assert!(parsed.get("health").is_some());
        assert!(parsed.get("listen_port").is_some());
        assert_eq!(parsed["enabled"], false);
    }
}
