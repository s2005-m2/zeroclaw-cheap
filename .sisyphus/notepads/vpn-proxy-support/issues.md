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


## VPN compile error: `crate::vpn` unresolved (T13 fix, 2026-02-26)

**Root cause:** `main.rs` and `lib.rs` both declare `mod tools`, but only `lib.rs` had `#[cfg(feature = "vpn")] pub mod vpn;`. When the binary crate compiled `tools/vpn_control.rs`, `crate::vpn` resolved against the binary crate root (which had no `vpn` module), causing unresolved import errors.

**Fix:** Added `#[cfg(feature = "vpn")] mod vpn;` to `src/main.rs` (line 85-86), matching the pattern used by all other shared modules. The `crate::vpn` imports in `vpn_control.rs` and `tools/mod.rs` were correct and did not need changing.

**Key learning:** This project has a dual-crate structure (lib + bin). Any module declared in `lib.rs` that is referenced via `crate::` from shared code must also be declared in `main.rs`. The compiler suggestion `help: a similar path exists: zeroclaw::vpn` was misleading — using `zeroclaw::` doesn't work from the binary crate either since it's the same package.