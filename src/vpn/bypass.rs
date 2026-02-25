//! Domestic traffic bypass for VPN proxy.
//!
//! Two-layer detection:
//! 1. Built-in domain suffix list (zero-latency fast path)
//! 2. IP geolocation API fallback via uapis.cn (for unknown domains)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// IP geolocation API endpoint (must NOT go through VPN).
const IP_GEO_API_URL: &str = "https://uapis.cn/api/v1/network/ipinfo";

/// API request timeout.
const API_TIMEOUT: Duration = Duration::from_secs(3);

/// LRU cache capacity for IP geolocation results.
const IP_CACHE_CAPACITY: usize = 10_000;

/// Cache entry TTL (24 hours).
const IP_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Built-in domestic domain suffixes.
///
/// Matching is suffix-based: `*.baidu.com` matches `www.baidu.com` and
/// `tieba.baidu.com`, but NOT bare `baidu.com`.
const BUILTIN_DOMESTIC_DOMAINS: &[&str] = &[
    ".baidu.com",
    ".bilibili.com",
    ".feishu.cn",
    ".larksuite.com",
    ".dingtalk.com",
    ".qq.com",
    ".weixin.qq.com",
    ".wechat.com",
    ".taobao.com",
    ".tmall.com",
    ".alipay.com",
    ".jd.com",
    ".douyin.com",
    ".zhihu.com",
    ".weibo.com",
    ".163.com",
    ".126.com",
    ".bytedance.com",
    ".xiaohongshu.com",
    ".meituan.com",
    ".didi.com",
    ".ctrip.com",
    ".aliyun.com",
    ".tencent.com",
    ".huawei.com",
    ".cn",
    ".com.cn",
];

/// Result of a bypass check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BypassDecision {
    /// Traffic should bypass VPN (domestic).
    Bypass,
    /// Traffic should go through VPN (foreign).
    Proxy,
    /// Could not determine — caller should treat as Proxy.
    Unknown,
}

/// Cached IP geolocation result with timestamp.
#[derive(Debug, Clone)]
struct CachedIpResult {
    decision: BypassDecision,
    inserted_at: Instant,
}

/// Simple LRU-ish cache backed by `HashMap` with timestamp-based eviction.
#[derive(Debug)]
struct IpCache {
    entries: HashMap<String, CachedIpResult>,
    capacity: usize,
    ttl: Duration,
}

impl IpCache {
    fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity / 4),
            capacity,
            ttl,
        }
    }

    fn get(&self, ip: &str) -> Option<BypassDecision> {
        let entry = self.entries.get(ip)?;
        if entry.inserted_at.elapsed() > self.ttl {
            return None;
        }
        Some(entry.decision)
    }

    fn insert(&mut self, ip: String, decision: BypassDecision) {
        // Evict expired entries when at capacity.
        if self.entries.len() >= self.capacity {
            let ttl = self.ttl;
            self.entries.retain(|_, v| v.inserted_at.elapsed() <= ttl);
        }
        // If still at capacity after eviction, remove oldest entry.
        if self.entries.len() >= self.capacity {
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(
            ip,
            CachedIpResult {
                decision,
                inserted_at: Instant::now(),
            },
        );
    }
}

/// Domestic traffic bypass checker.
///
/// Uses a two-layer approach:
/// 1. Domain suffix matching against a built-in + user-configured list (fast path).
/// 2. IP geolocation API fallback for unknown domains.
pub struct BypassChecker {
    /// Domain suffixes to match (stored as lowercase with leading dot).
    domain_suffixes: Vec<String>,
    /// Dedicated HTTP client that does NOT go through VPN.
    direct_client: reqwest::Client,
    /// LRU cache for IP geolocation results.
    ip_cache: Arc<RwLock<IpCache>>,
}

impl BypassChecker {
    /// Create a new bypass checker with built-in domains plus user extras.
    ///
    /// `extra_domains` accepts entries like `*.example.com`, `.example.com`,
    /// or bare `example.com` (which matches the exact domain).
    pub fn new(extra_domains: &[String]) -> Self {
        let mut suffixes: Vec<String> = BUILTIN_DOMESTIC_DOMAINS
            .iter()
            .map(|s| s.to_string())
            .collect();

        for raw in extra_domains {
            let normalized = Self::normalize_suffix(raw);
            if !normalized.is_empty() && !suffixes.contains(&normalized) {
                suffixes.push(normalized);
            }
        }

        // Build a direct (no-proxy) HTTP client for geo API queries.
        let direct_client = reqwest::Client::builder()
            .no_proxy()
            .timeout(API_TIMEOUT)
            .build()
            .expect("failed to build direct HTTP client");

        Self {
            domain_suffixes: suffixes,
            direct_client,
            ip_cache: Arc::new(RwLock::new(IpCache::new(IP_CACHE_CAPACITY, IP_CACHE_TTL))),
        }
    }

    /// Check if a domain matches the domestic bypass list.
    ///
    /// Returns `Bypass` if the domain suffix matches, `Proxy` otherwise.
    pub fn check_domain(&self, domain: &str) -> BypassDecision {
        let normalized = domain.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return BypassDecision::Unknown;
        }
        // Check suffix match: domain must end with one of the suffixes.
        // For suffix `.cn`, domain `example.cn` matches (ends with `.cn`).
        // For suffix `.baidu.com`, domain `www.baidu.com` matches.
        for suffix in &self.domain_suffixes {
            if normalized.ends_with(suffix.as_str()) {
                return BypassDecision::Bypass;
            }
            // Also match if the domain equals the suffix without leading dot.
            // e.g. suffix `.baidu.com` should match bare `baidu.com`.
            if let Some(bare) = suffix.strip_prefix('.') {
                if normalized == bare {
                    return BypassDecision::Bypass;
                }
            }
        }
        BypassDecision::Proxy
    }

    /// Query IP geolocation API to determine if an IP is domestic.
    ///
    /// Returns `Bypass` for Chinese IPs, `Proxy` for foreign, `Unknown` on
    /// timeout or API failure. Results are cached.
    pub async fn check_ip(&self, ip: &str) -> BypassDecision {
        // Check cache first.
        {
            let cache = self.ip_cache.read().await;
            if let Some(decision) = cache.get(ip) {
                return decision;
            }
        }
        // Query the geo API.
        let decision = self.query_ip_geo(ip).await;
        // Cache the result (skip Unknown to allow retry).
        if decision != BypassDecision::Unknown {
            let mut cache = self.ip_cache.write().await;
            cache.insert(ip.to_string(), decision);
        }
        decision
    }
    /// Combined check: domain fast path, then IP geo fallback.
    ///
    /// Returns `Bypass` if domain matches or IP is domestic.
    /// Returns `Proxy` if domain is foreign and IP is foreign.
    /// Returns `Unknown` only if domain check says Proxy and IP check fails.
    pub async fn should_bypass(&self, domain: &str, ip: Option<&str>) -> BypassDecision {
        let domain_result = self.check_domain(domain);
        if domain_result == BypassDecision::Bypass {
            return BypassDecision::Bypass;
        }
        // Domain not in list — try IP geo fallback if IP is provided.
        if let Some(ip) = ip {
            return self.check_ip(ip).await;
        }
        BypassDecision::Unknown
    }
    /// Add a domain suffix to the bypass list at runtime.
    pub fn add_domain(&mut self, domain: &str) {
        let normalized = Self::normalize_suffix(domain);
        if !normalized.is_empty() && !self.domain_suffixes.contains(&normalized) {
            self.domain_suffixes.push(normalized);
        }
    }

    /// Remove a domain suffix from the bypass list at runtime.
    pub fn remove_domain(&mut self, domain: &str) {
        let normalized = Self::normalize_suffix(domain);
        self.domain_suffixes.retain(|s| s != &normalized);
    }
    /// Generate a comma-separated `NO_PROXY`-style list from current suffixes.
    pub fn to_no_proxy_list(&self) -> String {
        self.domain_suffixes
            .iter()
            .map(|s| {
                // Convert `.example.com` to `*.example.com` for NO_PROXY format.
                if let Some(bare) = s.strip_prefix('.') {
                    format!("*.{bare}")
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }
    /// Query the IP geolocation API directly (no VPN).
    async fn query_ip_geo(&self, ip: &str) -> BypassDecision {
        let url = format!("{IP_GEO_API_URL}?ip={ip}");
        let resp = match self.direct_client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return BypassDecision::Unknown,
        };
        let body = match resp.text().await {
            Ok(t) => t,
            Err(_) => return BypassDecision::Unknown,
        };
        Self::parse_geo_response(&body)
    }
    /// Parse the geo API JSON response.
    ///
    /// Expected format: `{"country": "中国", ...}` or `{"country": "CN", ...}`.
    fn parse_geo_response(body: &str) -> BypassDecision {
        let json: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => return BypassDecision::Unknown,
        };
        let country = match json.get("country").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return BypassDecision::Unknown,
        };
        if country == "\u{4e2d}\u{56fd}" || country.eq_ignore_ascii_case("CN") || country == "China"
        {
            BypassDecision::Bypass
        } else {
            BypassDecision::Proxy
        }
    }
    /// Normalize a user-provided domain into a suffix with leading dot.
    ///
    /// `*.example.com` → `.example.com`
    /// `.example.com` → `.example.com`
    /// `example.com` → `.example.com`
    fn normalize_suffix(raw: &str) -> String {
        let trimmed = raw.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return String::new();
        }
        let without_star = trimmed.strip_prefix("*").unwrap_or(&trimmed);
        if without_star.starts_with('.') {
            without_star.to_string()
        } else {
            format!(".{without_star}")
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    // ── Domain matching ──────────────────────────────────────────

    #[test]
    fn builtin_domestic_domain_matches() {
        let checker = BypassChecker::new(&[]);
        assert_eq!(
            checker.check_domain("www.baidu.com"),
            BypassDecision::Bypass
        );
        assert_eq!(
            checker.check_domain("tieba.baidu.com"),
            BypassDecision::Bypass
        );
        assert_eq!(checker.check_domain("bilibili.com"), BypassDecision::Bypass);
        assert_eq!(
            checker.check_domain("api.feishu.cn"),
            BypassDecision::Bypass
        );
        assert_eq!(checker.check_domain("example.cn"), BypassDecision::Bypass);
        assert_eq!(checker.check_domain("test.com.cn"), BypassDecision::Bypass);
    }
    #[test]
    fn foreign_domain_does_not_match() {
        let checker = BypassChecker::new(&[]);
        assert_eq!(
            checker.check_domain("www.google.com"),
            BypassDecision::Proxy
        );
        assert_eq!(checker.check_domain("github.com"), BypassDecision::Proxy);
        assert_eq!(
            checker.check_domain("api.openai.com"),
            BypassDecision::Proxy
        );
        assert_eq!(checker.check_domain("example.org"), BypassDecision::Proxy);
    }
    #[test]
    fn extra_domain_is_added() {
        let checker = BypassChecker::new(&["*.custom.local".to_string()]);
        assert_eq!(
            checker.check_domain("app.custom.local"),
            BypassDecision::Bypass
        );
        assert_eq!(checker.check_domain("custom.local"), BypassDecision::Bypass);
    }
    #[test]
    fn case_insensitive_domain_matching() {
        let checker = BypassChecker::new(&[]);
        assert_eq!(
            checker.check_domain("WWW.BAIDU.COM"),
            BypassDecision::Bypass
        );
        assert_eq!(
            checker.check_domain("Www.Google.Com"),
            BypassDecision::Proxy
        );
    }
    #[test]
    fn empty_domain_returns_unknown() {
        let checker = BypassChecker::new(&[]);
        assert_eq!(checker.check_domain(""), BypassDecision::Unknown);
        assert_eq!(checker.check_domain("  "), BypassDecision::Unknown);
    }
    // ── IP geo response parsing ──────────────────────────────────
    #[test]
    fn parse_geo_response_chinese() {
        let body = r#"{"ip":"1.2.3.4","country":"中国","province":"北京"}"#;
        assert_eq!(
            BypassChecker::parse_geo_response(body),
            BypassDecision::Bypass
        );
    }
    #[test]
    fn parse_geo_response_cn_code() {
        let body = r#"{"ip":"1.2.3.4","country":"CN"}"#;
        assert_eq!(
            BypassChecker::parse_geo_response(body),
            BypassDecision::Bypass
        );
    }
    #[test]
    fn parse_geo_response_foreign() {
        let body = r#"{"ip":"8.8.8.8","country":"United States"}"#;
        assert_eq!(
            BypassChecker::parse_geo_response(body),
            BypassDecision::Proxy
        );
    }
    #[test]
    fn parse_geo_response_invalid_json() {
        assert_eq!(
            BypassChecker::parse_geo_response("not json"),
            BypassDecision::Unknown
        );
        assert_eq!(
            BypassChecker::parse_geo_response(""),
            BypassDecision::Unknown
        );
    }
    #[test]
    fn parse_geo_response_missing_country_field() {
        let body = r#"{"ip":"1.2.3.4","region":"Asia"}"#;
        assert_eq!(
            BypassChecker::parse_geo_response(body),
            BypassDecision::Unknown
        );
    }
    // ── Add/remove domain ──────────────────────────────────────
    #[test]
    fn add_and_remove_domain() {
        let mut checker = BypassChecker::new(&[]);
        assert_eq!(
            checker.check_domain("app.mysite.local"),
            BypassDecision::Proxy
        );
        checker.add_domain("*.mysite.local");
        assert_eq!(
            checker.check_domain("app.mysite.local"),
            BypassDecision::Bypass
        );
        checker.remove_domain("*.mysite.local");
        assert_eq!(
            checker.check_domain("app.mysite.local"),
            BypassDecision::Proxy
        );
    }
    // ── Cache behavior ──────────────────────────────────────────
    #[test]
    fn ip_cache_stores_and_retrieves() {
        let mut cache = IpCache::new(100, Duration::from_secs(3600));
        cache.insert("1.2.3.4".to_string(), BypassDecision::Bypass);
        assert_eq!(cache.get("1.2.3.4"), Some(BypassDecision::Bypass));
        assert_eq!(cache.get("5.6.7.8"), None);
    }
    #[test]
    fn ip_cache_expires_entries() {
        let mut cache = IpCache::new(100, Duration::from_millis(1));
        cache.insert("1.2.3.4".to_string(), BypassDecision::Bypass);
        // Sleep briefly to expire the entry.
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(cache.get("1.2.3.4"), None);
    }
    #[test]
    fn ip_cache_evicts_at_capacity() {
        let mut cache = IpCache::new(2, Duration::from_secs(3600));
        cache.insert("1.1.1.1".to_string(), BypassDecision::Bypass);
        cache.insert("2.2.2.2".to_string(), BypassDecision::Proxy);
        // At capacity — next insert should evict oldest.
        cache.insert("3.3.3.3".to_string(), BypassDecision::Bypass);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.get("3.3.3.3"), Some(BypassDecision::Bypass));
    }
    // ── NO_PROXY list ──────────────────────────────────────────────
    #[test]
    fn to_no_proxy_list_format() {
        let checker = BypassChecker::new(&[]);
        let list = checker.to_no_proxy_list();
        // Should contain wildcard-prefixed entries.
        assert!(list.contains("*.baidu.com"));
        assert!(list.contains("*.cn"));
        // Entries are comma-separated.
        assert!(list.contains(','));
    }
    // ── Normalize suffix ──────────────────────────────────────────
    #[test]
    fn normalize_suffix_variants() {
        assert_eq!(
            BypassChecker::normalize_suffix("*.example.com"),
            ".example.com"
        );
        assert_eq!(
            BypassChecker::normalize_suffix(".example.com"),
            ".example.com"
        );
        assert_eq!(
            BypassChecker::normalize_suffix("example.com"),
            ".example.com"
        );
        assert_eq!(
            BypassChecker::normalize_suffix("  *.TEST.COM  "),
            ".test.com"
        );
        assert_eq!(BypassChecker::normalize_suffix(""), "");
    }
    // ── Async should_bypass ─────────────────────────────────────
    #[tokio::test]
    async fn should_bypass_domain_fast_path() {
        let checker = BypassChecker::new(&[]);
        let result = checker.should_bypass("www.baidu.com", None).await;
        assert_eq!(result, BypassDecision::Bypass);
    }
    #[tokio::test]
    async fn should_bypass_foreign_no_ip_returns_unknown() {
        let checker = BypassChecker::new(&[]);
        let result = checker.should_bypass("www.google.com", None).await;
        assert_eq!(result, BypassDecision::Unknown);
    }
}
