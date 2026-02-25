//! Clash subscription parser for VPN proxy.
//!
//! Fetches and parses Clash proxy subscription URLs using the `subconverter`
//! crate for robust YAML parsing. Converts parsed proxies into a simplified
//! `ProxyNode` representation for downstream use by clash-lib.

use std::fmt;
use std::time::Duration;

/// Proxy node type parsed from Clash YAML.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NodeType {
    VMess,
    VLESS,
    Trojan,
    Shadowsocks,
    Http,
    Socks5,
    Other(String),
}

impl fmt::Display for NodeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VMess => write!(f, "vmess"),
            Self::VLESS => write!(f, "vless"),
            Self::Trojan => write!(f, "trojan"),
            Self::Shadowsocks => write!(f, "ss"),
            Self::Http => write!(f, "http"),
            Self::Socks5 => write!(f, "socks5"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

/// A single proxy node extracted from a Clash subscription.
///
/// `raw_config` preserves the full node configuration as JSON for later
/// consumption by clash-lib without data loss.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyNode {
    pub name: String,
    pub node_type: NodeType,
    pub server: String,
    pub port: u16,
    pub raw_config: serde_json::Value,
}

/// HTTP fetch timeout for subscription URLs.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Connect timeout for subscription fetch.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Clash subscription parser.
///
/// Fetches remote Clash YAML subscriptions and converts them into
/// `Vec<ProxyNode>` for use by the VPN runtime.
pub struct SubscriptionParser;

impl SubscriptionParser {
    /// Uses a direct (no-proxy) HTTP client for network access, since
    /// subscription URLs are expected to be reachable without VPN.
    /// This avoids the chicken-and-egg problem where the VPN proxy
    /// is not yet available when fetching the subscription for the first time.
    pub async fn fetch_and_parse(url: &str) -> anyhow::Result<Vec<ProxyNode>> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(FETCH_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build subscription HTTP client: {e}"))?;

        let resp = client
            .get(url)
            .header("User-Agent", "clash-verge/v2.0")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    anyhow::anyhow!("subscription fetch timed out: {url}")
                } else if e.is_connect() {
                    anyhow::anyhow!("failed to connect to subscription URL: {url}: {e}")
                } else {
                    anyhow::anyhow!("subscription fetch failed: {url}: {e}")
                }
            })?;

        let status = resp.status();
        if status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("subscription returned 403 Forbidden: {url}");
        }
        if !status.is_success() {
            anyhow::bail!("subscription returned HTTP {status}: {url}");
        }

        let content = resp
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("failed to read subscription response body: {e}"))?;

        Self::parse_clash_yaml(&content)
    }

    /// Parse raw Clash YAML content into proxy nodes.
    ///
    /// Uses the `subconverter` crate's Clash parser for robust deserialization,
    /// then converts each proxy into our `ProxyNode` representation.
    pub fn parse_clash_yaml(content: &str) -> anyhow::Result<Vec<ProxyNode>> {
        let proxies = libsubconverter::parser::yaml::clash::parse_clash_yaml(content)
            .map_err(|e| anyhow::anyhow!("invalid Clash YAML: {e}"))?;

        let nodes: Vec<ProxyNode> = proxies
            .into_iter()
            .filter(|p| p.proxy_type != libsubconverter::ProxyType::Unknown)
            .map(|p| proxy_to_node(p))
            .collect();

        if nodes.is_empty() {
            anyhow::bail!("no valid proxy nodes found in subscription");
        }

        Ok(nodes)
    }
}

/// Convert a subconverter `ProxyType` to our `NodeType`.
fn map_proxy_type(pt: libsubconverter::ProxyType) -> NodeType {
    match pt {
        libsubconverter::ProxyType::VMess => NodeType::VMess,
        libsubconverter::ProxyType::Vless => NodeType::VLESS,
        libsubconverter::ProxyType::Trojan => NodeType::Trojan,
        libsubconverter::ProxyType::Shadowsocks => NodeType::Shadowsocks,
        libsubconverter::ProxyType::HTTP | libsubconverter::ProxyType::HTTPS => NodeType::Http,
        libsubconverter::ProxyType::Socks5 => NodeType::Socks5,
        other => NodeType::Other(other.to_string().to_lowercase()),
    }
}

/// Convert a subconverter `Proxy` into our `ProxyNode`.
///
/// Serializes the full proxy to JSON for `raw_config` so clash-lib can
/// reconstruct the complete node configuration later.
fn proxy_to_node(proxy: libsubconverter::Proxy) -> ProxyNode {
    let name = proxy.remark.clone();
    let node_type = map_proxy_type(proxy.proxy_type);
    let server = proxy.hostname.clone();
    let port = proxy.port;

    // Serialize the full proxy struct to JSON for lossless downstream use.
    let raw_config = serde_json::to_value(&proxy).unwrap_or(serde_json::Value::Null);

    ProxyNode {
        name,
        node_type,
        server,
        port,
        raw_config,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

  - name: "vless-node"
    type: vless
    server: vless.example.com
    port: 443
    uuid: "a3482e88-686a-4a58-8126-99c9034e4b09"
    flow: xtls-rprx-vision
    tls: true

  - name: "http-node"
    type: http
    server: http.example.com
    port: 8080

  - name: "socks5-node"
    type: socks5
    server: socks.example.com
    port: 1080
"#;

    #[test]
    fn parse_clash_yaml_basic() {
        let nodes = SubscriptionParser::parse_clash_yaml(CLASH_YAML_FIXTURE).unwrap();
        assert_eq!(nodes.len(), 6);
    }

    #[test]
    fn parse_clash_yaml_node_types() {
        let nodes = SubscriptionParser::parse_clash_yaml(CLASH_YAML_FIXTURE).unwrap();

        let types: Vec<&NodeType> = nodes.iter().map(|n| &n.node_type).collect();
        assert_eq!(types[0], &NodeType::VMess);
        assert_eq!(types[1], &NodeType::Trojan);
        assert_eq!(types[2], &NodeType::Shadowsocks);
        assert_eq!(types[3], &NodeType::VLESS);
        assert_eq!(types[4], &NodeType::Http);
        assert_eq!(types[5], &NodeType::Socks5);
    }

    #[test]
    fn parse_clash_yaml_node_fields() {
        let nodes = SubscriptionParser::parse_clash_yaml(CLASH_YAML_FIXTURE).unwrap();

        let vmess = &nodes[0];
        assert_eq!(vmess.name, "vmess-node");
        assert_eq!(vmess.server, "vmess.example.com");
        assert_eq!(vmess.port, 443);
        assert!(!vmess.raw_config.is_null());

        let ss = &nodes[2];
        assert_eq!(ss.name, "ss-node");
        assert_eq!(ss.server, "ss.example.com");
        assert_eq!(ss.port, 8388);
    }

    #[test]
    fn parse_clash_yaml_raw_config_preserved() {
        let nodes = SubscriptionParser::parse_clash_yaml(CLASH_YAML_FIXTURE).unwrap();
        let vmess = &nodes[0];

        // raw_config should contain the full proxy data as JSON.
        assert!(vmess.raw_config.is_object());
        assert_eq!(
            vmess.raw_config.get("Hostname").and_then(|v| v.as_str()),
            Some("vmess.example.com")
        );
    }

    #[test]
    fn parse_clash_yaml_empty_proxies() {
        let yaml = "proxies: []";
        let result = SubscriptionParser::parse_clash_yaml(yaml);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no valid proxy nodes"));
    }

    #[test]
    fn parse_clash_yaml_invalid_yaml() {
        let result = SubscriptionParser::parse_clash_yaml("not: [valid: yaml: {{");
        assert!(result.is_err());
    }

    #[test]
    fn parse_clash_yaml_missing_proxies_key() {
        // ClashYamlInput defaults proxies to empty vec via serde(default).
        let yaml = "rules:\n  - DIRECT";
        let result = SubscriptionParser::parse_clash_yaml(yaml);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no valid proxy nodes"));
    }

    #[test]
    fn parse_clash_yaml_unknown_type_skipped() {
        let yaml = r#"
proxies:
  - name: "unknown-node"
    type: tuic
    server: tuic.example.com
    port: 443
  - name: "vmess-node"
    type: vmess
    server: vmess.example.com
    port: 443
    uuid: "b831381d-6324-4d53-ad4f-8cda48b30811"
    alterId: 0
    cipher: auto
"#;
        let nodes = SubscriptionParser::parse_clash_yaml(yaml).unwrap();
        // Unknown type (tuic) is filtered out, only vmess remains.
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_type, NodeType::VMess);
    }

    #[test]
    fn node_type_display() {
        assert_eq!(NodeType::VMess.to_string(), "vmess");
        assert_eq!(NodeType::VLESS.to_string(), "vless");
        assert_eq!(NodeType::Trojan.to_string(), "trojan");
        assert_eq!(NodeType::Shadowsocks.to_string(), "ss");
        assert_eq!(NodeType::Http.to_string(), "http");
        assert_eq!(NodeType::Socks5.to_string(), "socks5");
        assert_eq!(NodeType::Other("wireguard".into()).to_string(), "wireguard");
    }
}
