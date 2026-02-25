//! VPN ↔ ProxyConfig bridge.
//!
//! Connects the VPN runtime to ZeroClaw's proxy system via
//! `set_runtime_proxy_config()` / `runtime_proxy_config()`.

use anyhow::{bail, Result};
use std::sync::Mutex;

use super::bypass::BypassChecker;
use crate::config::{runtime_proxy_config, set_runtime_proxy_config, ProxyConfig, ProxyScope};

/// Bridge between VPN runtime and ZeroClaw's proxy configuration.
///
/// On `activate`, saves the current proxy config as backup, then installs a
/// VPN-specific config with the proxy URL and bypass domains merged into
/// `no_proxy`. On `deactivate`, restores the saved backup.
pub struct VpnProxyBridge {
    /// Backup of the proxy config before VPN activation (`None` = not active).
    backup: Mutex<Option<ProxyConfig>>,
}

impl VpnProxyBridge {
    /// Create a new inactive bridge.
    pub fn new() -> Self {
        Self {
            backup: Mutex::new(None),
        }
    }

    /// Activate VPN proxy: save current config as backup, then set VPN config.
    ///
    /// The new config uses `ProxyScope::Zeroclaw` so all ZeroClaw HTTP clients
    /// route through the VPN proxy, with bypass domains merged into `no_proxy`.
    pub fn activate(&self, proxy_url: &str, bypass_checker: &BypassChecker) -> Result<()> {
        let mut guard = self
            .backup
            .lock()
            .map_err(|e| anyhow::anyhow!("VPN bridge lock poisoned: {e}"))?;

        if guard.is_some() {
            bail!("VPN proxy bridge is already active; deactivate first");
        }

        // Save current proxy config as backup.
        let current = runtime_proxy_config();
        *guard = Some(current.clone());

        // Merge bypass domains with existing no_proxy entries.
        let bypass_list = bypass_checker.to_no_proxy_list();
        let merged_no_proxy = merge_no_proxy(&current.no_proxy, &bypass_list);

        // Build and install the VPN proxy config.
        let vpn_config = ProxyConfig {
            enabled: true,
            http_proxy: None,
            https_proxy: None,
            all_proxy: Some(proxy_url.to_string()),
            no_proxy: merged_no_proxy,
            scope: ProxyScope::Zeroclaw,
            services: Vec::new(),
        };

        set_runtime_proxy_config(vpn_config);
        Ok(())
    }

    /// Deactivate VPN proxy: restore the saved backup config.
    pub fn deactivate(&self) -> Result<()> {
        let mut guard = self
            .backup
            .lock()
            .map_err(|e| anyhow::anyhow!("VPN bridge lock poisoned: {e}"))?;

        let backup = match guard.take() {
            Some(cfg) => cfg,
            None => bail!("VPN proxy bridge is not active; nothing to deactivate"),
        };

        set_runtime_proxy_config(backup);
        Ok(())
    }

    /// Whether the VPN proxy bridge is currently active.
    pub fn is_active(&self) -> bool {
        self.backup.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// Update the proxy URL while the bridge is active.
    ///
    /// Re-reads the current runtime config, patches `all_proxy`, and re-sets it.
    /// Fails if the bridge is not active.
    pub fn update_proxy_url(&self, new_url: &str) -> Result<()> {
        let guard = self
            .backup
            .lock()
            .map_err(|e| anyhow::anyhow!("VPN bridge lock poisoned: {e}"))?;

        if guard.is_none() {
            bail!("VPN proxy bridge is not active; cannot update proxy URL");
        }

        let mut current = runtime_proxy_config();
        current.all_proxy = Some(new_url.to_string());
        set_runtime_proxy_config(current);
        Ok(())
    }
}

impl Default for VpnProxyBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge existing `no_proxy` entries with bypass checker's comma-separated list.
///
/// Deduplicates entries (case-insensitive) and preserves existing ones.
fn merge_no_proxy(existing: &[String], bypass_csv: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    // Keep existing entries first.
    for entry in existing {
        let key = entry.trim().to_ascii_lowercase();
        if !key.is_empty() && seen.insert(key) {
            result.push(entry.clone());
        }
    }

    // Append bypass entries that aren't already present.
    for entry in bypass_csv.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            result.push(trimmed.to_string());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: reset runtime proxy config to a known default state.
    fn reset_proxy_config() {
        set_runtime_proxy_config(ProxyConfig::default());
    }

    #[test]
    fn bridge_activate_sets_proxy() {
        reset_proxy_config();
        let bridge = VpnProxyBridge::new();
        let checker = BypassChecker::new(&[]);

        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();

        let cfg = runtime_proxy_config();
        assert!(cfg.enabled);
        assert_eq!(cfg.all_proxy.as_deref(), Some("socks5://127.0.0.1:7890"));
        assert_eq!(cfg.scope, ProxyScope::Zeroclaw);

        // Cleanup.
        bridge.deactivate().unwrap();
    }

    #[test]
    fn bridge_deactivate_restores_original() {
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
        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();

        // Config should now be VPN, not original.
        let mid = runtime_proxy_config();
        assert_eq!(mid.all_proxy.as_deref(), Some("socks5://127.0.0.1:7890"));

        // Deactivate should restore original.
        bridge.deactivate().unwrap();
        let restored = runtime_proxy_config();
        assert_eq!(restored.enabled, original.enabled);
        assert_eq!(restored.http_proxy, original.http_proxy);
        assert_eq!(restored.scope, original.scope);
        assert_eq!(restored.services, original.services);
        assert_eq!(restored.no_proxy, original.no_proxy);
    }
    #[test]
    fn bridge_bypass_in_no_proxy() {
        reset_proxy_config();
        let bridge = VpnProxyBridge::new();
        let checker = BypassChecker::new(&["*.custom.local".to_string()]);
        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();
        let cfg = runtime_proxy_config();
        // Bypass domains should appear in no_proxy.
        let joined = cfg.no_proxy.join(",");
        assert!(joined.contains("*.baidu.com"), "missing builtin bypass domain");
        assert!(joined.contains("*.custom.local"), "missing user bypass domain");
        assert!(joined.contains("*.cn"), "missing .cn TLD bypass");
        // Cleanup.
        bridge.deactivate().unwrap();
    }
    #[test]
    fn bridge_is_active_tracks_state() {
        reset_proxy_config();
        let bridge = VpnProxyBridge::new();
        let checker = BypassChecker::new(&[]);
        assert!(!bridge.is_active());
        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();
        assert!(bridge.is_active());
        bridge.deactivate().unwrap();
        assert!(!bridge.is_active());
    }
    #[test]
    fn bridge_update_proxy_url() {
        reset_proxy_config();
        let bridge = VpnProxyBridge::new();
        let checker = BypassChecker::new(&[]);
        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();
        assert_eq!(
            runtime_proxy_config().all_proxy.as_deref(),
            Some("socks5://127.0.0.1:7890")
        );
        bridge.update_proxy_url("socks5://127.0.0.1:9999").unwrap();
        assert_eq!(
            runtime_proxy_config().all_proxy.as_deref(),
            Some("socks5://127.0.0.1:9999")
        );
        // Cleanup.
        bridge.deactivate().unwrap();
    }
    #[test]
    fn bridge_no_proxy_merges_existing() {
        reset_proxy_config();
        // Set existing no_proxy entries before activation.
        let pre = ProxyConfig {
            enabled: false,
            http_proxy: None,
            https_proxy: None,
            all_proxy: None,
            no_proxy: vec!["localhost".to_string(), "*.internal.corp".to_string()],
            scope: ProxyScope::Zeroclaw,
            services: Vec::new(),
        };
        set_runtime_proxy_config(pre);
        let bridge = VpnProxyBridge::new();
        let checker = BypassChecker::new(&[]);
        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();
        let cfg = runtime_proxy_config();
        // Existing entries must be preserved.
        assert!(cfg.no_proxy.contains(&"localhost".to_string()));
        assert!(cfg.no_proxy.contains(&"*.internal.corp".to_string()));
        // Bypass entries must also be present.
        let joined = cfg.no_proxy.join(",");
        assert!(joined.contains("*.baidu.com"));
        // Cleanup.
        bridge.deactivate().unwrap();
    }
    #[test]
    fn bridge_activate_twice_fails() {
        reset_proxy_config();
        let bridge = VpnProxyBridge::new();
        let checker = BypassChecker::new(&[]);
        bridge.activate("socks5://127.0.0.1:7890", &checker).unwrap();
        let err = bridge.activate("socks5://127.0.0.1:9999", &checker);
        assert!(err.is_err());
        // Cleanup.
        bridge.deactivate().unwrap();
    }

    #[test]
    fn bridge_deactivate_when_inactive_fails() {
        let bridge = VpnProxyBridge::new();
        let err = bridge.deactivate();
        assert!(err.is_err());
    }

    #[test]
    fn bridge_update_when_inactive_fails() {
        let bridge = VpnProxyBridge::new();
        let err = bridge.update_proxy_url("socks5://127.0.0.1:9999");
        assert!(err.is_err());
    }

    #[test]
    fn merge_no_proxy_deduplicates() {
        let existing = vec!["localhost".to_string(), "*.baidu.com".to_string()];
        let bypass = "*.baidu.com,*.google.com";
        let merged = merge_no_proxy(&existing, bypass);
        // *.baidu.com should appear only once.
        let count = merged.iter().filter(|e| e == &&"*.baidu.com".to_string()).count();
        assert_eq!(count, 1);
        assert!(merged.contains(&"*.google.com".to_string()));
        assert!(merged.contains(&"localhost".to_string()));
    }
}
