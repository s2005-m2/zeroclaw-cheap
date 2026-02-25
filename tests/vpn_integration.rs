#![cfg(feature = "vpn")]
//! VPN module integration tests.
//!
//! Validates: subscription parsing, node cache roundtrip, bypass checker,
//! bridge activate/deactivate, node manager selection/failover, and tool schema.
//! All tests are self-contained — no network calls, no external services.

use std::time::Instant;

use zeroclaw::config::{runtime_proxy_config, set_runtime_proxy_config, ProxyConfig, ProxyScope};
use zeroclaw::vpn::{
    BypassChecker, BypassDecision, HealthResult, NodeCache, NodeManager, NodeStatus, NodeType,
    ProxyNode, SubscriptionParser, VpnProxyBridge,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Embedded Clash YAML fixture with multiple proxy types.
const CLASH_YAML_FIXTURE: &str = r#"
proxies:
  - name: "vmess-node"
    type: vmess
    server: vmess.example.com
    port: 443
    uuid: "b831381d-6324-4d53-ad4f-8cda48b30811"
    alterId: 0
    cipher: auto
    tls: true

  - name: "trojan-node"
    type: trojan
    server: trojan.example.com
    port: 443
    password: "trojan-password-placeholder"
    sni: trojan.example.com

  - name: "ss-node"
    type: ss
    server: ss.example.com
    port: 8388
    cipher: aes-256-gcm
    password: "ss-password-placeholder"
"#;

fn sample_nodes() -> Vec<ProxyNode> {
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
            checked_at: Instant::now(),
        },
    )
}

/// Reset runtime proxy config to default state (for bridge tests).
fn reset_proxy_config() {
    set_runtime_proxy_config(ProxyConfig::default());
}

// ─────────────────────────────────────────────────────────────────────────────
// A. Subscription Parser Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn parse_clash_yaml_extracts_nodes() {
    let nodes = SubscriptionParser::parse_clash_yaml(CLASH_YAML_FIXTURE).unwrap();
    assert_eq!(nodes.len(), 3);

    assert_eq!(nodes[0].name, "vmess-node");
    assert_eq!(nodes[0].server, "vmess.example.com");
    assert_eq!(nodes[0].port, 443);
    assert_eq!(nodes[0].node_type, NodeType::VMess);

    assert_eq!(nodes[1].name, "trojan-node");
    assert_eq!(nodes[1].server, "trojan.example.com");
    assert_eq!(nodes[1].node_type, NodeType::Trojan);

    assert_eq!(nodes[2].name, "ss-node");
    assert_eq!(nodes[2].server, "ss.example.com");
    assert_eq!(nodes[2].port, 8388);
    assert_eq!(nodes[2].node_type, NodeType::Shadowsocks);
}

#[test]
fn parse_clash_yaml_preserves_raw_config() {
    let nodes = SubscriptionParser::parse_clash_yaml(CLASH_YAML_FIXTURE).unwrap();
    for node in &nodes {
        assert!(
            node.raw_config.is_object(),
            "raw_config for '{}' should be a JSON object",
            node.name
        );
    }
}

#[test]
fn parse_clash_yaml_empty_rejects() {
    let result = SubscriptionParser::parse_clash_yaml("proxies: []");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("no valid proxy nodes"));
}

#[test]
fn parse_clash_yaml_invalid_rejects() {
    let result = SubscriptionParser::parse_clash_yaml("not: [valid: yaml: {{");
    assert!(result.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// B. Node Cache Roundtrip
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn node_cache_save_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vpn").join("nodes.json");

    let nodes = sample_nodes();
    NodeCache::save(&nodes, &path).await.unwrap();

    let loaded = NodeCache::load(&path).await.unwrap().unwrap();
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded[0].name, "node-a");
    assert_eq!(loaded[0].node_type, NodeType::VMess);
    assert_eq!(loaded[0].server, "a.example.com");
    assert_eq!(loaded[0].port, 443);
    assert_eq!(loaded[1].name, "node-b");
    assert_eq!(loaded[1].node_type, NodeType::Trojan);
    assert_eq!(loaded[2].name, "node-c");
    assert_eq!(loaded[2].node_type, NodeType::Shadowsocks);
    assert_eq!(loaded[2].port, 8388);
}

#[tokio::test]
async fn node_cache_load_nonexistent_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("does-not-exist.json");
    let result = NodeCache::load(&path).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn node_cache_load_corrupt_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.json");
    tokio::fs::write(&path, "not valid json {{{").await.unwrap();
    let result = NodeCache::load(&path).await.unwrap();
    assert!(result.is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// C. Bypass Checker
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn bypass_checker_domestic_domains() {
    let checker = BypassChecker::new(&[]);
    assert_eq!(
        checker.check_domain("www.baidu.com"),
        BypassDecision::Bypass
    );
    assert_eq!(
        checker.check_domain("api.bilibili.com"),
        BypassDecision::Bypass
    );
    assert_eq!(checker.check_domain("example.cn"), BypassDecision::Bypass);
    assert_eq!(checker.check_domain("test.com.cn"), BypassDecision::Bypass);
    // Foreign domains should NOT be bypassed.
    assert_eq!(
        checker.check_domain("www.google.com"),
        BypassDecision::Proxy
    );
    assert_eq!(checker.check_domain("github.com"), BypassDecision::Proxy);
}

#[test]
fn bypass_checker_custom_domains() {
    let checker = BypassChecker::new(&["*.custom.local".to_string()]);
    assert_eq!(
        checker.check_domain("app.custom.local"),
        BypassDecision::Bypass
    );
    assert_eq!(checker.check_domain("custom.local"), BypassDecision::Bypass);
    // Unrelated domain still proxied.
    assert_eq!(
        checker.check_domain("other.example.org"),
        BypassDecision::Proxy
    );
}

#[test]
fn bypass_checker_add_remove_domain() {
    let mut checker = BypassChecker::new(&[]);
    assert_eq!(
        checker.check_domain("app.mysite.dev"),
        BypassDecision::Proxy
    );
    checker.add_domain("*.mysite.dev");
    assert_eq!(
        checker.check_domain("app.mysite.dev"),
        BypassDecision::Bypass
    );
    checker.remove_domain("*.mysite.dev");
    assert_eq!(
        checker.check_domain("app.mysite.dev"),
        BypassDecision::Proxy
    );
}

#[test]
fn bypass_checker_no_proxy_list_format() {
    let checker = BypassChecker::new(&[]);
    let list = checker.to_no_proxy_list();
    assert!(list.contains("*.baidu.com"));
    assert!(list.contains("*.cn"));
    assert!(list.contains(','));
}

#[test]
fn bypass_checker_empty_domain_returns_unknown() {
    let checker = BypassChecker::new(&[]);
    assert_eq!(checker.check_domain(""), BypassDecision::Unknown);
    assert_eq!(checker.check_domain("  "), BypassDecision::Unknown);
}
// ─────────────────────────────────────────────────────────────────────────────
// D. Bridge Activate/Deactivate
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn bridge_activate_deactivate_preserves_config() {
    reset_proxy_config();
    // Set a known pre-activation config.
    let original = ProxyConfig {
        enabled: true,
        http_proxy: Some("http://corp-proxy:8080".to_string()),
        https_proxy: None,
        all_proxy: None,
        no_proxy: vec!["localhost".to_string()],
        scope: ProxyScope::Services,
        services: vec!["provider.*".to_string()],
    };
    set_runtime_proxy_config(original.clone());

    let bridge = VpnProxyBridge::new();
    let checker = BypassChecker::new(&[]);
    bridge
        .activate("socks5://127.0.0.1:7890", &checker)
        .unwrap();

    // While active, config should reflect VPN proxy.
    let mid = runtime_proxy_config();
    assert_eq!(mid.all_proxy.as_deref(), Some("socks5://127.0.0.1:7890"));
    assert_eq!(mid.scope, ProxyScope::Zeroclaw);
    assert!(mid.enabled);

    // Deactivate should restore original.
    bridge.deactivate().unwrap();
    let restored = runtime_proxy_config();
    assert_eq!(restored.enabled, original.enabled);
    assert_eq!(restored.http_proxy, original.http_proxy);
    assert_eq!(restored.scope, original.scope);
    assert_eq!(restored.services, original.services);
    assert_eq!(restored.no_proxy, original.no_proxy);
}
// ─────────────────────────────────────────────────────────────────────────────
// E. Node Manager Selection
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn node_manager_selects_lowest_latency() {
    let mgr = NodeManager::new(sample_nodes());
    let health = vec![
        make_health("node-a", NodeStatus::Healthy, Some(100)),
        make_health("node-b", NodeStatus::Healthy, Some(50)),
        make_health("node-c", NodeStatus::Healthy, Some(200)),
    ];
    let best = mgr.select_best_node(&health).unwrap();
    assert_eq!(best.name, "node-b");
}
#[test]
fn node_manager_skips_unhealthy() {
    let mgr = NodeManager::new(sample_nodes());
    let health = vec![
        make_health("node-a", NodeStatus::Unhealthy, None),
        make_health("node-b", NodeStatus::Healthy, Some(80)),
        make_health("node-c", NodeStatus::Unhealthy, None),
    ];
    let best = mgr.select_best_node(&health).unwrap();
    assert_eq!(best.name, "node-b");
}
#[test]
fn node_manager_all_unhealthy_returns_none() {
    let mgr = NodeManager::new(sample_nodes());
    let health = vec![
        make_health("node-a", NodeStatus::Unhealthy, None),
        make_health("node-b", NodeStatus::Unhealthy, None),
        make_health("node-c", NodeStatus::Unknown, None),
    ];
    assert!(mgr.select_best_node(&health).is_none());
}
#[test]
fn node_manager_failover_skips_active() {
    let mut mgr = NodeManager::new(sample_nodes());
    mgr.set_active("node-b");
    let health = vec![
        make_health("node-a", NodeStatus::Healthy, Some(100)),
        make_health("node-b", NodeStatus::Healthy, Some(50)),
        make_health("node-c", NodeStatus::Healthy, Some(80)),
    ];
    let next = mgr.failover(&health).unwrap();
    // node-b is skipped (active), node-c (80ms) beats node-a (100ms).
    assert_eq!(next.name, "node-c");
    assert_eq!(mgr.active_node().unwrap().name, "node-c");
}
#[test]
fn node_manager_set_active_and_all_nodes() {
    let mut mgr = NodeManager::new(sample_nodes());
    assert!(mgr.active_node().is_none());
    assert_eq!(mgr.all_nodes().len(), 3);
    assert!(mgr.set_active("node-c"));
    assert_eq!(mgr.active_node().unwrap().name, "node-c");
    assert!(!mgr.set_active("no-such-node"));
}
// ─────────────────────────────────────────────────────────────────────────────
// F. Tool Schema Validation
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn vpn_control_tool_schema_valid() {
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use zeroclaw::security::{AutonomyLevel, SecurityPolicy};
    use zeroclaw::tools::traits::Tool;
    use zeroclaw::tools::VpnControlTool;
    use zeroclaw::tools::VpnState;
    use zeroclaw::vpn::NodeManager;

    let security = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    });
    let state = Arc::new(RwLock::new(VpnState {
        runtime: None,
        node_manager: NodeManager::new(vec![]),
        bypass_checker: BypassChecker::new(&[]),
        bridge: VpnProxyBridge::new(),
        health_cancel: None,
        last_health: vec![],
        subscription_url: None,
        listen_port: 7890,
        health_check_interval_secs: 30,
    }));
    let tool = VpnControlTool::new(security, state);

    assert_eq!(tool.name(), "vpn_control");
    assert!(!tool.description().is_empty());

    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["action"]["enum"].is_array());
    assert_eq!(schema["required"][0], "action");
}
// ─────────────────────────────────────────────────────────────────────────────
// G. Feature Flag Isolation
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn vpn_module_compiles_with_feature() {
    // Verify core VPN types are accessible under the vpn feature flag.
    let _checker = BypassChecker::new(&[]);
    let _manager = NodeManager::new(vec![]);
    let _bridge = VpnProxyBridge::new();
    let _status_display = NodeStatus::Healthy.to_string();
    assert_eq!(_status_display, "healthy");
    let _node_type_display = NodeType::VMess.to_string();
    assert_eq!(_node_type_display, "vmess");
}
