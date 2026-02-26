# Hook System — Learnings

## 2026-02-26 Initial Analysis
- Existing hooks: `src/hooks/traits.rs` (HookHandler trait, 9 void + 6 modifying hooks), `runner.rs` (HookRunner with priority sort + panic safety), `mod.rs` (re-exports), `builtin/` (command_logger)
- HookRunner currently uses `Vec<Box<dyn HookHandler>>` — no Arc/RwLock yet (Task 7 adds this)
- HooksConfig at schema.rs:1898 has only `enabled: bool` and `builtin: BuiltinHooksConfig` — Task 2 extends this
- MigrateCommands enum at lib.rs:167 is the pattern for CLI subcommands
- SkillManifest at skills/mod.rs:73 is the pattern for TOML manifest parsing
- Config stamp polling pattern at channels/mod.rs:614-682 is the pattern for hot-reload
- Windows build: `cargo check --bin zeroclaw` works, `cargo test --lib` may hit LNK1318 PDB limit


## Task 4: HooksCommands CLI enum + dispatch stub
- `Commands` enum lives in `src/main.rs` (line ~137), NOT in `src/lib.rs`
- Command enums (ServiceCommands, SkillCommands, etc.) live in `src/lib.rs`
- In `Commands` enum, variants reference lib types as `zeroclaw::HooksCommands` (qualified path)
- Dispatch match block is in `src/main.rs` around line ~990-1025
- `cargo check --bin zeroclaw` is the fast verification command on Windows
- HooksCommands placed after SkillCommands (line 163) in lib.rs, Hooks variant after Skills in Commands enum

## Task 1: HookManifest schema and parser
- Used `HookManifestWrapper` (private) with `[hook]` top-level key, mirroring SKILL.toml's `[skill]` pattern
- `HookEvent` enum uses `#[serde(rename_all = "snake_case")]` so TOML values are `on_session_start` etc.
- `HookAction` enum uses same serde rename — TOML uses `[hook.action.shell]`, `[hook.action.http]`, `[hook.action.prompt_inject]`
- Validation runs inside `from_toml()` after deserialization — checks empty names, empty commands/urls/content, zero timeouts
- 9 tests written: shell/http/prompt_inject parsing, missing event, invalid event, zero timeout, conditions, defaults, Display impl
- `cargo check --bin zeroclaw` passes; only warning is unused `pub use manifest::*` (expected — downstream tasks will use it)

## Task 3: HookAuditReport + audit_hook_directory
- Mirrored `src/skills/audit.rs` pattern: `HookAuditReport` with `findings: Vec<String>`, `files_scanned: usize`, `is_clean()`, `summary()`
- `audit_hook_directory(hook_dir, skip_security_audit)` — skip flag returns clean report immediately (zero files scanned)
- Checks: symlinks blocked, path traversal (.. and null bytes), file size limit (512KB), HOOK.toml presence required
- Dangerous pattern detection via `OnceLock<Vec<(Regex, &str)>>`: fork bombs, reverse shells (/dev/tcp, nc -e, bash -i), curl|sh, wget|sh, rm -rf /, mkfs, dd if=
- Shell chaining detection: `&&`/`||`/`;`/`|` followed by dangerous commands
- HOOK.toml parsed inline with `toml::Value` (no dependency on HookManifest from Task 1)
- 8 tests: safe hook, curl|sh, fork bomb, symlink, skip bypass, missing HOOK.toml, reverse shell, rm -rf
- `cargo check --bin zeroclaw` passes clean (only pre-existing `manifest::*` warning)

## Task 5+6: loader.rs + dynamic.rs (hook directory loader + DynamicHookHandler)
- `LoadedHook` struct: `manifest: HookManifest` + `hook_dir: PathBuf`
- `load_hooks_from_dir(hooks_dir, config)`: walks subdirs for HOOK.toml, parses via `HookManifest::from_toml()`, runs `audit_hook_directory()`, skips invalid hooks with warning, enforces `max_hooks`, sorts by priority descending
- `DynamicHookHandler` wraps `LoadedHook` + `default_timeout_secs`, implements all 15 HookHandler methods
- Void hooks: check `matches_event()` then `fire_void_action()` (tokio::spawn fire-and-forget)
- Modifying hooks: check event match + conditions, then execute action or apply PromptInject inline
- Shell execution: `tokio::process::Command` with platform-aware shell (`cmd /C` on Windows, `sh -c` on Unix), timeout via `tokio::time::timeout`
- HTTP execution: `reqwest::Client` with configurable method/headers/body/timeout
- PromptInject: prepend (default) or append content to prompt string, handled inline without spawning
- Condition evaluation: best-effort channel/user/pattern matching; skips check when no context available
- Had to fix `runner.rs` modifying dispatchers: T7 migration left `run_on_message_sending` still using old `self.handlers` field; fixed to use `static_handlers`/`dynamic_handlers` chain pattern
- Also fixed `runner.rs` test `register_and_sort_by_priority` to use `static_handlers` instead of `handlers`
- 7 tests in loader.rs: single hook, multiple sorted, malformed skip, max_hooks, empty dir, nonexistent dir error, non-directory skip
- 8 tests in dynamic.rs: name/priority, shell on match, non-match noop, timeout, prompt inject prepend, channel condition filter, disabled noop, empty command graceful
- `cargo check --bin zeroclaw` and `cargo check --tests` both pass clean

## Task 7: HookRunner runtime registry swap (static/dynamic split)
- Split `handlers: Vec<Box<dyn HookHandler>>` into `static_handlers` (compile-time) + `dynamic_handlers: Arc<RwLock<Vec<Box<dyn HookHandler>>>>` (hot-reloadable)
- `register()` adds to `static_handlers` only (never swapped)
- `reload_dynamic_hooks(&self, hooks)` acquires write lock, sorts by priority, atomically replaces dynamic vec
- Void dispatchers: acquire read lock, chain `static_handlers.iter()` + `dynamic.iter()`, `join_all` futures
- Modifying dispatchers: acquire read lock, merge both lists into `Vec<&dyn HookHandler>`, sort by priority, iterate sequentially with `AssertUnwindSafe` + `catch_unwind`
- Key design choice: modifying dispatchers build a merged sorted vec per call (not pre-sorted) because dynamic handlers can change between calls
- `reload_dynamic_hooks` takes `&self` (not `&mut self`) — only needs shared ref since RwLock handles interior mutability
- 3 new tests: `reload_swaps_dynamic_handlers`, `static_handlers_unaffected_by_reload`, `empty_reload_clears_dynamic`
- `empty_reload_clears_dynamic` test uses `let runner` (not `let mut runner`) since reload only needs `&self`
- `cargo check --bin zeroclaw` and `cargo check --tests` both pass clean
- Windows LNK1318 PDB limit still blocks `cargo test --lib` linking (known issue, not related to our changes)