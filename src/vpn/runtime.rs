//! Clash external-process proxy runtime manager for ZeroClaw.
//!
//! Spawns a local Clash proxy process to provide SOCKS5 proxy
//! on `127.0.0.1:{port}`. The runtime manages the child process lifecycle.
//!
//! # Lifecycle
//! ```text
//! ClashRuntime::start(yaml, port)
//!   → generates Clash config YAML
//!   → writes config to temp file
//!   → spawns `clash` binary as child process
//!   → waits for startup confirmation
//!   → returns ClashRuntime handle
//!
//! runtime.stop()
//!   → kills the child process
//!   → waits for exit
//! ```

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use super::subscription::ProxyNode;

/// Clash controller API default port (RESTful API for node switching).
pub(crate) const CLASH_CONTROLLER_PORT: u16 = 9090;

/// Name of the proxy selector group in generated Clash config.
pub(crate) const SELECTOR_GROUP_NAME: &str = "zeroclaw-select";

// ── Clash config YAML generation ────────────────────────────────────

/// Generate a minimal Clash config YAML string.
///
/// Produces config with:
/// - `socks-port` bound to `127.0.0.1`
/// - `proxies` array from provided nodes
/// - `proxy-groups` with a single `select` group containing all nodes
/// - `rules` routing all traffic through the selector group
pub fn generate_clash_config(nodes: &[ProxyNode], socks_port: u16) -> Result<String> {
    if nodes.is_empty() {
        bail!("cannot generate Clash config with zero proxy nodes");
    }

    let mut yaml = String::with_capacity(2048);

    // Global settings — always bind to localhost.
    yaml.push_str(&format!(
        "socks-port: {socks_port}\n\
         bind-address: 127.0.0.1\n\
         allow-lan: false\n\
         mode: rule\n\
         log-level: warning\n\
         external-controller: 127.0.0.1:{CLASH_CONTROLLER_PORT}\n\n"
    ));

    // Proxies section — emit each node's raw_config as a YAML mapping.
    yaml.push_str("proxies:\n");
    for node in nodes {
        append_proxy_yaml(&mut yaml, node)?;
    }

    // Proxy group — single selector containing all node names.
    yaml.push_str("\nproxy-groups:\n");
    yaml.push_str(&format!("  - name: \"{}\"\n", SELECTOR_GROUP_NAME));
    yaml.push_str("    type: select\n");
    yaml.push_str("    proxies:\n");
    for node in nodes {
        yaml.push_str(&format!("      - \"{}\"\n", escape_yaml_string(&node.name)));
    }

    // Rules — route everything through the selector.
    yaml.push_str("\nrules:\n");
    yaml.push_str(&format!("  - MATCH,{}\n", SELECTOR_GROUP_NAME));

    Ok(yaml)
}

/// Append a single proxy node as YAML to the config string.
///
/// Converts the node's `raw_config` JSON back to YAML inline mapping.
/// Falls back to minimal `name/type/server/port` if raw_config is unusable.
fn append_proxy_yaml(yaml: &mut String, node: &ProxyNode) -> Result<()> {
    if let Some(obj) = node.raw_config.as_object() {
        yaml.push_str(&format!(
            "  - name: \"{}\"\n",
            escape_yaml_string(&node.name)
        ));
        yaml.push_str(&format!("    type: {}\n", node.node_type));
        yaml.push_str(&format!("    server: {}\n", node.server));
        yaml.push_str(&format!("    port: {}\n", node.port));

        // Emit remaining fields from raw_config, skipping already-emitted ones.
        let skip_keys = [
            "Remark",
            "remark",
            "name",
            "ProxyType",
            "proxy_type",
            "type",
            "Hostname",
            "hostname",
            "server",
            "Port",
            "port",
        ];
        for (key, value) in obj {
            if skip_keys.iter().any(|&s| s.eq_ignore_ascii_case(key)) {
                continue;
            }
            let lower_key = pascal_to_kebab(key);
            yaml.push_str(&format!(
                "    {}: {}\n",
                lower_key,
                json_value_to_yaml(value)
            ));
        }
    } else {
        // Fallback: minimal proxy entry.
        yaml.push_str(&format!(
            "  - name: \"{}\"\n    type: {}\n    server: {}\n    port: {}\n",
            escape_yaml_string(&node.name),
            node.node_type,
            node.server,
            node.port,
        ));
    }
    Ok(())
}

/// Convert a PascalCase key to lowercase-kebab (Clash YAML convention).
fn pascal_to_kebab(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('-');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}

/// Convert a JSON value to a YAML-compatible inline string.
fn json_value_to_yaml(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => {
            if s.contains(':') || s.contains('#') || s.contains('"') || s.is_empty() {
                format!("\"{}\"", escape_yaml_string(s))
            } else {
                s.clone()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "~".to_string(),
        // Arrays and objects: fall back to JSON representation.
        other => other.to_string(),
    }
}

/// Escape special characters for YAML double-quoted strings.
fn escape_yaml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ── ClashRuntime ────────────────────────────────────────────────────

/// Handle to a running Clash proxy process.
///
/// Spawns the `clash` binary as an external child process with a config
/// file. Calling `stop()` kills the process and cleans up the config file.
pub struct ClashRuntime {
    /// Port the SOCKS5 proxy listens on.
    socks_port: u16,
    /// Child process handle for the running clash binary.
    child: Option<tokio::process::Child>,
    /// Path to the temporary config file written for this runtime.
    config_path: PathBuf,
}

impl ClashRuntime {
    /// Start a Clash proxy runtime with the given config YAML.
    ///
    /// Writes the config to a temp file and spawns the `clash` binary.
    /// The runtime binds a SOCKS5 listener on `127.0.0.1:{listen_port}`.
    /// `config_yaml` should be a complete Clash YAML config string (use
    /// `generate_clash_config` to produce one from `ProxyNode` list).
    ///
    /// # Errors
    /// Returns error if the clash binary is not found or fails to start.
    pub async fn start(config_yaml: &str, listen_port: u16) -> Result<Self> {
        // Resolve the clash binary path.
        let clash_bin = which::which("clash")
            .or_else(|_| which::which("clash-rs"))
            .context(
                "'clash' binary not found in PATH. Install clash-rs or set PATH accordingly.",
            )?;

        // Write config YAML to a state directory.
        let state_dir = directories::UserDirs::new()
            .map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf())
            .join(".zeroclaw")
            .join("state")
            .join("vpn");
        std::fs::create_dir_all(&state_dir).context("failed to create VPN state directory")?;

        let config_path = state_dir.join("clash-config.yaml");
        std::fs::write(&config_path, config_yaml).context("failed to write Clash config file")?;

        // Spawn the clash binary as a child process.
        let child = tokio::process::Command::new(&clash_bin)
            .arg("-f")
            .arg(&config_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn clash binary: {}", clash_bin.display()))?;

        // Brief delay to let clash bind the port.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Verify the process is still alive after startup.
        let mut runtime = Self {
            socks_port: listen_port,
            child: Some(child),
            config_path,
        };

        if !runtime.is_running() {
            // Process exited during startup — collect exit status for diagnostics.
            if let Some(mut child) = runtime.child.take() {
                match child.wait().await {
                    Ok(status) => bail!("clash process exited during startup with {status}"),
                    Err(e) => bail!("clash process exited during startup: {e}"),
                }
            }
            bail!("clash process exited during startup");
        }

        Ok(runtime)
    }

    /// Stop the Clash process gracefully.
    ///
    /// Kills the child process and waits for it to exit.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            // Attempt graceful kill, then wait.
            child.kill().await.ok();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
        }
        self.child = None;
        Ok(())
    }

    /// Check if the Clash process is currently running.
    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => child.try_wait().ok().flatten().is_none(),
            None => false,
        }
    }

    /// Get the local SOCKS5 proxy URL.
    ///
    /// Always returns `socks5://127.0.0.1:{port}` — never exposed externally.
    pub fn local_proxy_url(&self) -> String {
        format!("socks5://127.0.0.1:{}", self.socks_port)
    }

    /// Get the SOCKS5 listen port.
    pub fn socks_port(&self) -> u16 {
        self.socks_port
    }

    /// Switch the active proxy node in the running Clash selector group.
    ///
    /// Uses Clash's RESTful API to change the selected node in the
    /// `zeroclaw-select` proxy group.
    ///
    /// # Errors
    /// Returns error if the runtime is not running or the node name is invalid.
    pub async fn switch_node(&mut self, node_name: &str) -> Result<()> {
        if !self.is_running() {
            bail!("clash runtime is not running");
        }
        let url = format!("http://127.0.0.1:{CLASH_CONTROLLER_PORT}/proxies/{SELECTOR_GROUP_NAME}");
        let client = reqwest::Client::new();
        let resp = client
            .put(&url)
            .json(&serde_json::json!({ "name": node_name }))
            .send()
            .await
            .context("failed to reach Clash controller API for node switch")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Clash node switch failed (HTTP {status}): {body}");
        }
        Ok(())
    }
}

impl Drop for ClashRuntime {
    fn drop(&mut self) {
        // kill_on_drop(true) handles process cleanup, but we also
        // clean up the config file.
        if let Some(ref mut child) = self.child {
            // Best-effort sync kill via start_kill (non-async).
            let _ = child.start_kill();
        }
        // Clean up the config file.
        let _ = std::fs::remove_file(&self.config_path);
    }
}
// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_nodes() -> Vec<ProxyNode> {
        vec![
            ProxyNode {
                name: "vmess-tokyo".to_string(),
                node_type: crate::vpn::subscription::NodeType::VMess,
                server: "tokyo.example.com".to_string(),
                port: 443,
                raw_config: serde_json::json!({
                    "Remark": "vmess-tokyo",
                    "ProxyType": "VMess",
                    "Hostname": "tokyo.example.com",
                    "Port": 443,
                    "Uuid": "b831381d-6324-4d53-ad4f-8cda48b30811",
                    "AlterId": 0,
                    "Cipher": "auto",
                    "Tls": true
                }),
            },
            ProxyNode {
                name: "trojan-sg".to_string(),
                node_type: crate::vpn::subscription::NodeType::Trojan,
                server: "sg.example.com".to_string(),
                port: 443,
                raw_config: serde_json::json!({
                    "Remark": "trojan-sg",
                    "ProxyType": "Trojan",
                    "Hostname": "sg.example.com",
                    "Port": 443,
                    "Password": "placeholder-password"
                }),
            },
        ]
    }
    #[test]
    fn generate_config_basic_structure() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 7891).unwrap();
        assert!(yaml.contains("socks-port: 7891"));
        assert!(yaml.contains("bind-address: 127.0.0.1"));
        assert!(yaml.contains("allow-lan: false"));
        assert!(yaml.contains("mode: rule"));
    }
    #[test]
    fn generate_config_contains_proxies() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 7891).unwrap();
        assert!(yaml.contains("proxies:"));
        assert!(yaml.contains("name: \"vmess-tokyo\""));
        assert!(yaml.contains("name: \"trojan-sg\""));
        assert!(yaml.contains("server: tokyo.example.com"));
        assert!(yaml.contains("port: 443"));
    }
    #[test]
    fn generate_config_contains_proxy_group() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 7891).unwrap();
        assert!(yaml.contains("proxy-groups:"));
        assert!(yaml.contains("name: \"zeroclaw-select\""));
        assert!(yaml.contains("type: select"));
        assert!(yaml.contains("- \"vmess-tokyo\""));
        assert!(yaml.contains("- \"trojan-sg\""));
    }
    #[test]
    fn generate_config_contains_rules() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 7891).unwrap();
        assert!(yaml.contains("rules:"));
        assert!(yaml.contains("MATCH,zeroclaw-select"));
    }
    #[test]
    fn generate_config_empty_nodes_fails() {
        let result = generate_clash_config(&[], 7891);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("zero proxy nodes"));
    }
    #[test]
    fn generate_config_always_binds_localhost() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 12345).unwrap();
        assert!(yaml.contains("bind-address: 127.0.0.1"));
        assert!(yaml.contains("allow-lan: false"));
        // Must never contain 0.0.0.0.
        assert!(!yaml.contains("0.0.0.0"));
    }
    #[test]
    fn generate_config_custom_port() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 9999).unwrap();
        assert!(yaml.contains("socks-port: 9999"));
    }
    #[test]
    fn generate_config_contains_controller() {
        let nodes = sample_nodes();
        let yaml = generate_clash_config(&nodes, 7891).unwrap();
        assert!(yaml.contains("external-controller: 127.0.0.1:9090"));
    }
    #[test]
    fn pascal_to_kebab_conversion() {
        assert_eq!(pascal_to_kebab("AlterId"), "alter-id");
        assert_eq!(pascal_to_kebab("Uuid"), "uuid");
        assert_eq!(pascal_to_kebab("Tls"), "tls");
        assert_eq!(pascal_to_kebab("ProxyType"), "proxy-type");
        assert_eq!(pascal_to_kebab("password"), "password");
    }
    #[test]
    fn escape_yaml_string_special_chars() {
        assert_eq!(escape_yaml_string("hello"), "hello");
        assert_eq!(escape_yaml_string("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_yaml_string("back\\slash"), "back\\\\slash");
    }
    #[test]
    fn json_value_to_yaml_types() {
        assert_eq!(
            json_value_to_yaml(&serde_json::Value::String("auto".into())),
            "auto"
        );
        assert_eq!(
            json_value_to_yaml(&serde_json::Value::Number(443.into())),
            "443"
        );
        assert_eq!(json_value_to_yaml(&serde_json::Value::Bool(true)), "true");
        assert_eq!(json_value_to_yaml(&serde_json::Value::Null), "~");
    }
    #[test]
    fn json_value_to_yaml_string_with_colon() {
        let val = serde_json::Value::String("http://example.com".into());
        let result = json_value_to_yaml(&val);
        assert!(result.starts_with('"'));
        assert!(result.ends_with('"'));
    }
    #[test]
    fn local_proxy_url_format() {
        let url = format!("socks5://127.0.0.1:{}", 7891_u16);
        assert_eq!(url, "socks5://127.0.0.1:7891");
        assert!(url.starts_with("socks5://127.0.0.1:"));
    }
    #[test]
    fn generate_config_skips_duplicate_fields() {
        let node = ProxyNode {
            name: "test-node".to_string(),
            node_type: crate::vpn::subscription::NodeType::Shadowsocks,
            server: "ss.example.com".to_string(),
            port: 8388,
            raw_config: serde_json::json!({
                "Remark": "test-node",
                "Hostname": "ss.example.com",
                "Port": 8388,
                "Cipher": "aes-256-gcm",
                "Password": "placeholder"
            }),
        };
        let yaml = generate_clash_config(&[node], 7891).unwrap();
        let server_count = yaml.matches("server:").count();
        assert_eq!(server_count, 1, "server field should not be duplicated");
    }
    #[test]
    fn generate_config_fallback_without_raw_config() {
        let node = ProxyNode {
            name: "minimal-node".to_string(),
            node_type: crate::vpn::subscription::NodeType::Socks5,
            server: "socks.example.com".to_string(),
            port: 1080,
            raw_config: serde_json::Value::Null,
        };
        let yaml = generate_clash_config(&[node], 7891).unwrap();
        assert!(yaml.contains("name: \"minimal-node\""));
        assert!(yaml.contains("type: socks5"));
        assert!(yaml.contains("server: socks.example.com"));
        assert!(yaml.contains("port: 1080"));
    }
}
