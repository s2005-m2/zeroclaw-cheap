# Draft: VPN/Proxy Smart Routing Support

## Requirements (confirmed)
- User wants ZeroClaw to manage VPN connections (on/off, node switching)
- Use subconverter-rs as embedded Rust library to parse Clash subscription URLs
- Expose `clash_proxy_url` in config for subscription URL
- Auto-failover: if VPN causes network loss, auto-switch node or disable VPN
- Selective proxy: agent's outbound calls can optionally use VPN
- Domestic services (飞书/百度/B站 etc.) should NOT go through VPN
- Non-agent networking should NOT go through VPN
- Node selection: latency-priority (periodic speed test, pick lowest latency)
- Agent control: full control via new tool (on/off, node list, node switch, status)
- Domestic bypass: built-in defaults (from external service/list) + agent can configure at runtime

## Technical Decisions
- Integration: subconverter-rs as crate dependency (zero process overhead)
- Existing proxy system is mature — extend it, don't replace it
- reqwest already has `socks` feature enabled
- Existing `proxy_config` tool handles proxy get/set — new VPN tool will complement it
- ProxyConfig already supports per-service routing via `scope: services` + service selectors
- Client caching layer already invalidates on proxy config change

## Research Findings (from explore agent)
- All providers use `build_runtime_proxy_client` or `apply_runtime_proxy_to_builder` with service keys
- Channels (discord, lark, whatsapp, signal, dingtalk) use proxy-aware clients
- GAP: `web_search_tool.rs` creates clients WITHOUT proxy integration
- GAP: OAuth blocking clients (MiniMax, Qwen) don't apply proxy
- GAP: telegram, slack, matrix channels need proxy audit
- Architecture: no HTTP client in traits — each impl handles its own client
- Client cache key: `service_key|timeout=X|connect_timeout=Y`

## Open Questions
- subconverter-rs: which crate exactly? (crates.io name / github repo)
- Clash subscription: standard Clash YAML format?
- Health check: what interval? (e.g. 30s, 60s, 5min?)
- Should VPN feature be behind a cargo feature flag?
- Node persistence: store parsed nodes in memory only, or persist to disk?

## Scope Boundaries
- INCLUDE: (to be confirmed)
- EXCLUDE: (to be confirmed)
