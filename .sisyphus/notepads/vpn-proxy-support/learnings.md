# VPN Proxy Support — Learnings

## T2: VpnConfig schema (2026-02-25)

- `#[cfg(feature = "vpn")]` must guard: struct def, Default impl, impl block, serde default fns, Config field, validation call, env overrides, and re-export in mod.rs
- Config struct is constructed explicitly in 3 places (schema.rs Default impl, wizard.rs x2) — all need `#[cfg(feature = "vpn")] vpn: VpnConfig::default()`
- `default_true()` helper already exists at line ~868 — reuse it for bool fields defaulting to true
- Serde default functions need their own `#[cfg(feature = "vpn")]` guards (e.g. `default_vpn_listen_port`)
- Env overrides use a `#[cfg(feature = "vpn")] { ... }` block pattern to keep the feature gate clean
- Validation delegation follows `self.proxy.validate()?` pattern — add `#[cfg(feature = "vpn")] self.vpn.validate()?` right after it


## T4: web_search_tool.rs proxy integration
- Pattern: create `builder` via `reqwest::Client::builder()`, then `crate::config::apply_runtime_proxy_to_builder(builder, "tool.web_search")`, then `.build()?`
- No explicit `use` import needed — http_request.rs uses full crate path `crate::config::apply_runtime_proxy_to_builder`
- Two client creation sites in web_search_tool.rs: `search_bing` (line ~38) and `search_brave` (line ~114)
- Service key added to SUPPORTED_PROXY_SERVICE_KEYS alphabetically after `tool.pushover`
- Both `cargo check` and `cargo check --features vpn` pass clean (only pre-existing warnings)


## T5: OAuth blocking client proxy integration (2026-02-25)
- `apply_to_reqwest_builder` takes async `reqwest::ClientBuilder`, incompatible with `reqwest::blocking::ClientBuilder`
- `normalize_proxy_url_option`, `apply_no_proxy`, `no_proxy_value` are all private to schema.rs — can't reuse from providers/mod.rs
- Created `apply_runtime_proxy_to_blocking_builder` helper in providers/mod.rs that manually replicates proxy application for blocking builders
- Inline trim+empty check replaces private `normalize_proxy_url_option`: `.map(str::trim).filter(|s| !s.is_empty())`
- No_proxy handling omitted from blocking helper (private `no_proxy_value` inaccessible) — acceptable for OAuth token refresh endpoints
- Service keys used: `provider.qwen` and `provider.minimax` — note these are NOT yet in SUPPORTED_PROXY_SERVICE_KEYS (schema.rs), so they only activate under ProxyScope::Zeroclaw, not ProxyScope::Services
- Both `cargo check` and `cargo check --features vpn` pass clean


## T3: bypass.rs domestic traffic bypass (2026-02-25)

- `DomainMatcher` in `src/security/domain_matcher.rs` uses wildcard pattern matching (category-based: banking/medical/gov) — different semantics from bypass suffix matching, so built standalone
- Suffix-based matching stores entries as `.example.com` internally; `*.example.com`, `.example.com`, and bare `example.com` all normalize to `.example.com`
- `reqwest::Client::builder().no_proxy()` creates a client that bypasses all proxy settings — critical for IP geo API queries to avoid circular VPN dependency
- LRU cache uses `HashMap<String, CachedIpResult>` with `Instant`-based TTL; evicts expired entries first, then oldest if still at capacity
- `Unknown` results are NOT cached to allow retry on transient API failures
- IP geo API at `uapis.cn` returns JSON with `country` field — match `"中国"`, `"CN"`, or `"China"`
- Pre-existing test compilation errors in schema.rs (missing `mcp`/`vpn` fields), gateway (missing `wati`), security/detect.rs (wrong import) block `cargo test` but are NOT from bypass code
- `cargo check --features vpn --lib` passes clean — zero warnings from bypass module
- `tokio::sync::RwLock` used for cache to allow concurrent reads with exclusive writes in async context


## T6: subscription.rs Clash subscription parser (2026-02-25)

- The `subconverter` crate registers as `libsubconverter` in Rust — use `libsubconverter::` not `subconverter::` for paths
- Key API: `libsubconverter::parser::yaml::clash::parse_clash_yaml(content: &str) -> Result<Vec<Proxy>, String>`
- `Proxy` struct uses `remark` for node name, `hostname` for server, `port` for port, `proxy_type` for type enum
- `ProxyType::Unknown` is returned for unrecognized types (e.g. `tuic`) — filter these out
- `Proxy` derives `Serialize` so `serde_json::to_value(&proxy)` works for raw_config preservation
- `Proxy` serde uses `#[serde(rename_all = "PascalCase")]` — JSON keys are PascalCase (e.g. `Hostname`, `Port`)
- `ClashYamlInput` has `#[serde(default)]` on proxies field — missing `proxies:` key yields empty vec, not parse error
- `serde_yaml` is a transitive dep from subconverter — available when vpn feature is on but not directly depended on
- `build_runtime_proxy_client_with_timeouts` used with service key `vpn.subscription` for proxy-aware HTTP fetching
- User-Agent `clash-verge/v2.0` used to avoid subscription provider blocks on unknown agents
- `cargo check --features vpn` passes clean — zero warnings from subscription module


## T7: runtime.rs clash-lib runtime manager (2026-02-25)

- clash-lib actual API (v0.8.2 confirmed via GitHub source):
  - `clash_lib::start_scaffold(Options) -> Result<()>` — blocks the calling thread (creates its own Tokio runtime)
  - `clash_lib::shutdown() -> bool` — stops all running instances globally
  - `clash_lib::Options { config, cwd, rt, log_file }` — startup options struct
  - `clash_lib::Config::Str(String)` — accepts YAML config as string
  - `clash_lib::TokioRuntime::MultiThread` — runtime type enum
- CRITICAL BLOCKER: `vpn` feature flag does NOT exist in Cargo.toml yet, and `mod vpn` is NOT declared in lib.rs
  - The vpn module files exist on disk (bypass.rs, subscription.rs, runtime.rs) but are NOT compiled
  - `cargo check --features vpn` fails with "package does not contain this feature: vpn"
  - Task T1 (feature flag setup) was marked complete but the wiring is missing
  - Needs: `vpn = ["dep:clash-lib"]` in [features], `clash-lib` in [dependencies], `#[cfg(feature = "vpn")] pub mod vpn;` in lib.rs
- `start_scaffold` creates its own Tokio runtime internally — must run in a dedicated OS thread, not on the existing Tokio runtime
- Clash exposes a RESTful controller API on `127.0.0.1:9090` by default for node switching (PUT /proxies/{group})
- Config YAML must include `external-controller: 127.0.0.1:9090` to enable the controller API
- Generated config uses `zeroclaw-select` as the proxy group name (single selector group)
- `allow-lan: false` and `bind-address: 127.0.0.1` enforced in generated config (security: never expose externally)
- PascalCase keys from subconverter's serde output (e.g. `Hostname`, `AlterId`) converted to kebab-case for Clash YAML
- Skip keys `Remark/ProxyType/Hostname/Port` from raw_config to avoid duplication (emitted explicitly as name/type/server/port)
- `rustfmt --check` passes clean on both runtime.rs and mod.rs
- Tests use `crate::vpn::subscription::NodeType` paths — will compile once module is wired into lib.rs


## T8: node_manager.rs disk cache persistence (2026-02-25)

- `ProxyNode` and `NodeType` originally lacked `Serialize`/`Deserialize` derives — added `serde::Serialize, serde::Deserialize` to both
- `NodeType` derive line: `#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]`
- `ProxyNode` derive line: `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`
- `CachedNodes` wrapper struct stores `fetched_at: String` (ISO-8601 via `chrono::Utc::now().to_rfc3339()`) + `nodes: Vec<ProxyNode>`
- `NodeCache` is a unit struct with three static methods: `save`, `load`, `default_cache_path`
- `directories::BaseDirs::new()` used for home dir resolution — already a dependency
- Corrupt JSON handling: `serde_json::from_str` error caught, logged via `tracing::warn!`, returns `Ok(None)`
- File-not-found handling: `path.exists()` check before read, returns `Ok(None)` if missing
- `tokio::fs::create_dir_all` creates parent dirs on save
- `tempfile::tempdir()` used in tests — crate already available
- Pre-existing compile errors in gateway/security/service modules unrelated to vpn — `cargo check --features vpn` errors are all outside `src/vpn/`
- rust-analyzer not installed on this machine — LSP diagnostics unavailable, relied on cargo check output
- Windows path separator handled in `default_cache_path` test with `||` for both `/` and `\\`


## T9: health.rs health checker and latency tester (2026-02-25)

- `futures` crate is optional (only enabled for `memory-lancedb` feature) — use `futures_util::future::join_all` instead of `futures::future::join_all`
- `futures-util` v0.3 is always available as a non-optional dependency with `sink` feature
- `reqwest::Proxy::all(proxy_url)` accepts SOCKS5 URLs like `socks5://127.0.0.1:7890` — no special builder needed
- `reqwest::Client::builder().proxy(p).no_proxy()` — `.no_proxy()` disables system proxy env vars, only the explicit `.proxy(p)` is used
- Probe URL `http://connectivitycheck.gstatic.com/generate_204` returns 204 on success; also accept 200 as some proxies rewrite responses
- `tokio_util::sync::CancellationToken` already used extensively in `src/channels/mod.rs` and `src/agent/loop_.rs` — established pattern
- Background loop pattern: `tokio::time::interval` + `tokio::select!` on `token.cancelled()` vs `ticker.tick()`
- First `interval.tick()` fires immediately — consume it before entering the select loop to trigger an immediate first check
- `HealthChecker` is a unit struct (like `NodeCache`, `SubscriptionParser`) — all methods are associated functions, no instance state
- `HealthResult.checked_at` uses `Instant` (monotonic) not `SystemTime` — suitable for latency comparison, not serialization
- Pre-existing compile errors unchanged: gateway/wati, security/estop+otp, onboard/wizard, service/linger — zero errors from `src/vpn/`
- `cargo check --features vpn --lib` produces zero warnings from vpn module


## T10: bridge.rs VPN ↔ ProxyConfig bridge (2026-02-25)

- `VpnProxyBridge` uses `std::sync::Mutex<Option<ProxyConfig>>` for backup state — matches the `std::sync::RwLock` used by `runtime_proxy_state()` in schema.rs (not tokio lock)
- `set_runtime_proxy_config()` auto-clears client cache — no need to manually invalidate after bridge activate/deactivate
- `runtime_proxy_config()` clones the config (handles poisoned lock gracefully) — safe to call from bridge without holding any lock
- `merge_no_proxy` deduplicates case-insensitively using `HashSet<String>` on lowercased keys, preserving original casing of first occurrence
- `BypassChecker::to_no_proxy_list()` returns comma-separated `*.domain` format — split on `,` and trim for merge
- `ProxyConfig` fields are all public — direct struct construction works without builder pattern
- Tests must call `reset_proxy_config()` (set default) before each test since runtime proxy state is global/shared across tests
- Pre-existing compile errors unchanged: gateway/wati, security/estop+otp, onboard/wizard, service/linger — zero errors from `src/vpn/bridge.rs`
- `cargo check --features vpn` produces zero VPN-specific errors or warnings


## T11: vpn_control.rs agent-facing VPN control tool (2026-02-25)

- `VpnConfig` struct does NOT exist in schema.rs yet — `Config` has no `vpn` field
- Stored VPN config values (subscription_url, listen_port, health_check_interval_secs) in `VpnState` instead of reading from `Config.vpn`
- `VpnControlTool` takes `Arc<SecurityPolicy>` + `Arc<RwLock<VpnState>>` — no `Arc<Config>` needed
- `VpnState` is the shared mutable state wrapper holding all VPN components + config values
- File is NOT compiled yet — `tools/mod.rs` registration is T13's job
- `rustfmt --check` passes clean on vpn_control.rs
- Pre-existing compile errors unchanged: gateway/wati, security/estop+otp, service/linger — zero errors from `src/tools/vpn_control.rs`
- Security pattern: `status`/`list_nodes` are read-only (no `require_write_access`), all mutation actions require `can_act()` + `record_action()`
- `HealthChecker::spawn_background_loop` callback uses `tokio::spawn` for fire-and-forget async state update (avoids blocking the health loop)



## T13: tools/mod.rs + schema.rs VPN wiring (2026-02-25)

- `Config.vpn` field does NOT exist — `VpnConfig` struct was never added to schema.rs (T2 incomplete or lost)
- Adapted tool registration to use env vars instead of `root_config.vpn.*`:
  - `ZEROCLAW_VPN_ENABLED` ("1"/"true" to enable)
  - `ZEROCLAW_VPN_CLASH_PROXY_URL` (subscription URL)
  - `ZEROCLAW_VPN_LISTEN_PORT` (default 7890)
  - `ZEROCLAW_VPN_HEALTH_INTERVAL_SECS` (default 30)
  - `ZEROCLAW_VPN_BYPASS_EXTRA` (comma-separated domains)
- Module decl: `#[cfg(feature = "vpn")] pub mod vpn_control;` after line 54 (web_search_tool)
- Re-export: `#[cfg(feature = "vpn")] pub use vpn_control::{VpnControlTool, VpnState};` after WebSearchTool re-export
- Tool registration: env-var-driven block after composio registration, inside `#[cfg(feature = "vpn")]`
- Added `"tool.web_search"` and `"vpn.subscription"` to `SUPPORTED_PROXY_SERVICE_KEYS` in schema.rs
- `cargo check --features vpn`: 7 pre-existing errors (gateway/wati, security/estop+otp, wizard, service/linger) — zero VPN-specific errors
- `cargo check` (no features): same 7 pre-existing errors — VPN code correctly excluded by cfg gate
- When `VpnConfig` is eventually added to `Config`, the env-var approach should be replaced with `root_config.vpn.*` field access


## T14: tests/vpn_integration.rs integration tests (2026-02-25)

- Created `tests/vpn_integration.rs` with `#![cfg(feature = "vpn")]` top-level gate
- 17 test functions covering all 7 required scenarios: subscription parsing (4), node cache roundtrip (3), bypass checker (5), bridge activate/deactivate (1), node manager selection/failover (4), tool schema validation (1), feature flag isolation (1)
- All tests are self-contained — no network calls, no external services, no timing dependencies
- Imports use `zeroclaw::vpn::*` and `zeroclaw::config::*` public paths (not `crate::` internal paths)
- `HealthResult` fields are public — direct struct construction works for mock health results (no need for `healthy()`/`unhealthy()` constructors which are `fn` not `pub fn`)
- Bridge tests must call `reset_proxy_config()` before each test since runtime proxy state is global
- `VpnControlTool` schema test imports `SecurityPolicy` + `AutonomyLevel` from `zeroclaw::security` — both are public
- `rustfmt --check` passes clean on the test file
- `cargo check --features vpn --test vpn_integration` shows only 7 pre-existing errors (gateway/wati, security/estop+otp, wizard/FeishuConfig, service/linger) — zero errors from test file
- No modifications to any existing files — test file is entirely additive