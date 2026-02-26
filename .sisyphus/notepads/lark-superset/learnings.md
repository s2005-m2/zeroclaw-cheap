## Notepad: Lark Superset — Learnings

<!-- Append-only. Each entry timestamped. -->

## T1: Hook System Extension (on_cron_delivery, on_docs_sync_notify)

- Modifying hooks follow a 4-tuple pattern: `(source/file_path, channel, recipient, content)` -> `HookResult<(String, String, String, String)>`
- Runner dispatcher pattern: collect static+dynamic handlers, sort by priority desc, loop with AssertUnwindSafe+catch_unwind, pipe Continue values, short-circuit on Cancel
- DynamicHookHandler pattern: check matches_event -> check_conditions -> execute_action -> return Continue/warn
- HookEvent enum uses serde `rename_all = snake_case` so no separate FromStr impl needed
- Pre-existing warnings (4): unused imports in hooks/mod.rs and tools/mod.rs — these are known and should be ignored
- rust-analyzer not installed on this machine; use `cargo check` as definitive compilation verification

## T3: Cron Delivery — Lark/Feishu Arms + Hook Integration

- `deliver_if_configured()` signature extended with `hook_runner: Option<&crate::hooks::HookRunner>` — callers pass `None` for now (cron scheduler doesn't have a HookRunner in scope)
- Lark arm uses `LarkChannel::from_lark_config()`, Feishu arm uses `LarkChannel::from_feishu_config()` — these are the canonical constructors from `lark.rs`
- Config access: `config.channels_config.lark` for lark, `config.channels_config.feishu` for feishu
- Both arms gated behind `#[cfg(feature = "channel-lark")]`
- Hook flow: `run_on_cron_delivery` before send (Cancel blocks delivery), `fire_message_sent` after successful send
- Pre-existing issue found: `LarkPlatform` enum was used in `lark.rs` but never defined — added it with methods: `api_base`, `ws_base`, `channel_name`, `proxy_service_key`, `locale_header`

# Lark CardKit Draft Streaming — Learnings

## Patterns
- `LarkChannel` struct derives `Clone` — any `Mutex` fields must be wrapped in `Arc`
- `StreamMode` enum already exists in `config::schema` (shared with Telegram)
- `default_draft_update_interval_ms()` returns 1000 — Lark uses 500ms minimum for CardKit
- URL helpers follow pattern: `fn xxx_url(&self) -> String { format!("{}/path", self.api_base()) }`
- All API calls use `send_text_once()` + token refresh pattern
- `from_config`/`from_lark_config`/`from_feishu_config` all need field wiring when struct changes
- `wizard.rs` constructs configs with explicit fields — must add new fields there too
- No rust-analyzer on this Windows env — rely on `cargo check` for validation

## T7: feishu-docs-sync module

### Patterns
- Adding a new field to `Config` struct requires updating 3 places: `schema.rs` default impl, and 2 `wizard.rs` initializers
- Re-exports in `config/mod.rs` are alphabetically sorted — inserting a new type shifts subsequent items and can displace them
- `notify` crate v6 uses `recommended_watcher()` with closure-based `EventHandler`, not trait impl
- Feature-gated modules use `#[cfg(feature = "...")]` in `lib.rs` — same pattern as `vpn`
- Tenant token caching pattern: `Arc<RwLock<Option<CachedToken>>>` with proactive refresh before expiry

### Gotchas
- When editing re-export lists line-by-line, each line replacement can displace items from the next line — cascading fixes needed
- `crate::config::DocsSyncConfig` won't resolve unless added to `config/mod.rs` re-exports
- Pre-existing 4 unused import warnings are expected — do not try to fix them

## T8: feishu-docs-sync-guide.md documentation

### Patterns
- SUMMARY.md uses `### N) Section Name` headings with flat bullet lists of relative links
- Operations & Deployment (section 3) is the right home for setup/sync guides
- Docs follow a consistent structure: Overview, Prerequisites, Config, Security, Troubleshooting
- Hook trait signatures use 4-tuple `(file_path, channel, recipient, content)` with `HookResult::Cancel` for suppression
- DocsSyncConfig defaults: 5 sync files, 60s interval, disabled by default, no auto-create
- Security rejection in sync.rs is all-or-nothing per sync cycle, not per-section