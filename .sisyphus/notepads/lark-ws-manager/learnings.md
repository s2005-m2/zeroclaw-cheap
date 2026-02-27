# Learnings — lark-ws-manager

## Task 1: Create lark_ws_manager.rs

- `lark.rs` has 108 pre-existing compile errors on `main` (broken `impl` blocks, missing types). These are NOT caused by our changes.
- `lark_ws_manager.rs` compiles cleanly under both `channel-lark` and `feishu-docs-sync` feature flags — zero errors from the new module.
- `build_runtime_proxy_client` lives in `src/config/schema.rs` and takes a `service_key: &str` param. Used `"lark"` as the key.
- `WS_HEARTBEAT_TIMEOUT` in lark.rs is 90s; the task spec says 300s for the manager. Used 300s as specified.
- `should_refresh_last_recv` matches on `Binary | Ping | Pong` — identical in both lark.rs and event_subscriber.rs.
- PbFrame/PbHeader/WsClientConfig/WsEndpointResp/WsEndpoint are identical between lark.rs and event_subscriber.rs (confirmed by reading both).
- Fragment reassembly fix: `seq_num >= sum` is treated as single-frame (not discarded), matching the existing lark.rs behavior at line 740.
- The `select! biased` ordering is: heartbeat tick > timeout check > msg read — preserved from lark.rs.
- No rust-analyzer available on this Windows machine; used `cargo build` for verification.
- Module registered with `#[cfg(any(feature = "channel-lark", feature = "feishu-docs-sync"))]` to cover both consumers.


## Task 3: Refactor event_subscriber.rs to use LarkWsManager

- File went from 349 lines to 91 lines — all WS connection logic removed.
- Deleted: `PbHeader`, `PbFrame`, `WsClientConfig`, `WsEndpointResp`, `WsEndpoint`, `FEISHU_WS_BASE_URL`, `WS_HEARTBEAT_TIMEOUT`.
- Deleted: `get_ws_endpoint()`, `listen_ws()` methods.
- Kept: `DriveEvent`, `DriveEventHeader` (docs_sync-specific business types).
- `EventSubscriber` now holds `Arc<LarkWsManager>` + `document_id` only.
- `run()` subscribes to manager broadcast, filters `drive.file.edit_v1`, preserves document_id filtering.
- `broadcast::error::RecvError::Lagged` is the correct path in tokio (not `broadcast::RecvError`).
- `LarkWsEvent` import removed — type is inferred from `rx.recv()`, avoids unused-import warning.
- Expected compile error in `worker.rs:169` (old `new()` signature) — Task 4 will fix.
- No rust-analyzer on this machine; verified with `cargo check --features feishu-docs-sync`.

## Task 2: Refactor lark.rs to subscribe to LarkWsManager

- File went from 2955 lines to 2700 lines — all raw WS connection logic removed.
- Deleted: `PbHeader`, `PbFrame`, `WsClientConfig`, `WsEndpointResp`, `WsEndpoint` struct definitions.
- Deleted: `should_refresh_last_recv()`, `WS_HEARTBEAT_TIMEOUT`, `get_ws_endpoint()` method.
- Deleted two test functions that tested the now-removed `should_refresh_last_recv`.
- Removed imports: `futures_util`, `prost::Message`, `tokio_tungstenite`.
- Added imports: `lark_ws_manager::{LarkWsEvent, LarkWsManager}`, `tokio::sync::broadcast`.
- Added `ws_manager: Option<Arc<LarkWsManager>>` field + setter + constructor init.
- `listen_ws()` rewritten to subscribe to broadcast, handles `Lagged`/`Closed` errors.
- All business logic preserved: dedup, allowlist, content parsing, ACK reaction, overflow buffer, group @-mention gating, auto-share docs.
- Kept `FEISHU_WS_BASE_URL`, `LARK_WS_BASE_URL` constants — still used by tests.
- Pre-existing extra `}` at EOF and broken nested `impl` blocks confirmed in original file.

## Task 4: Wire LarkWsManager through daemon startup flow

- `start_channels` has 3 callers: `daemon/mod.rs:52`, `main.rs:738`, `main.rs:994`.
- `run_worker` has 1 caller: `daemon/mod.rs:99` (re-exported via `docs_sync/mod.rs:16`).
- `#[cfg]` on function parameters works in Rust — used to conditionally add `lark_ws_manager` param.
- `main.rs` callers pass `None` for the manager (no shared WS needed outside daemon).
- `daemon/mod.rs` creates the manager BEFORE channels and docs_sync supervisors, spawns `mgr.run()` via `tokio::spawn`.
- Feishu config takes precedence over lark config for credential resolution (is_feishu=true).
- Added `#[allow(unused_variables)]` to `start_channels` for the feishu-docs-sync-only case.
- `docs_sync/mod.rs` re-export auto-propagates signature changes — no edit needed.
- `channel-lark` only build still fails with pre-existing `lark.rs:2700` extra `}` — not from Task 4.
- `feishu-docs-sync` only and no-features builds both succeed with zero new warnings.