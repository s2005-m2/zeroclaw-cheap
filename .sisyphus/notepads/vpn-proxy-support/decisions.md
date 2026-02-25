# VPN Proxy Support — Decisions

## Architecture
- Proxy runtime: clash-lib (clash-rs) embedded Rust library
- Subscription parsing: subconverter-rs as crate dependency (GPL-3.0 accepted)
- Feature flag: `--features vpn` cargo feature
- Node selection: latency-priority with 30s health checks
- Domestic bypass: domain list fast path + IP geo API fallback (uapis.cn)
- Node persistence: disk cache at ~/.zeroclaw/state/vpn/
- Proxy integration: via existing set_runtime_proxy_config() API
