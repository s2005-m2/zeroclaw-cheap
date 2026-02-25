# VPN Proxy Support — Issues

## T7: clash-lib removal — resolved
- `clash-lib` (v0.8.2 and v0.9.4) both fail to compile due to watfaq-dns/hickory-server API mismatch
- Replaced with external process approach: spawn `clash` binary via `tokio::process::Command`
- `clash-lib` removed from Cargo.toml; `subconverter` kept for subscription parsing
- `vpn` feature updated to `vpn = ["dep:subconverter"]`
- `is_running()` signature changed from `&self` to `&mut self` (needed for `try_wait()`)
- `switch_node()` signature changed from `&self` to `&mut self` (calls `is_running()`)
- Pre-existing errors in gateway/onboard/security/service modules unrelated to vpn changes
- All existing tests pass (they test config generation, not clash-lib)
