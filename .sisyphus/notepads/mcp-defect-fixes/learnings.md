## Notepad: MCP Defect Fixes — Learnings

<!-- Append-only. Each entry timestamped. -->

### 2026-02-25: MAX_PAGES pagination guard
- All three pagination loops (`list_tools`, `list_resources`, `list_prompts`) share identical structure: loop → send → receive → parse → extend → check nextCursor → break/continue.
- Placed `const MAX_PAGES: usize = 100;` at module level (near struct def) rather than inside methods, since it's shared across three methods.
- Pre-existing compile errors found in the package: `transport.rs:403` had a missing `{}` format specifier in `error!()` macro. Had to fix to get tests to compile.
- MockTransport test pattern: queue `MAX_PAGES + 1` responses each with `nextCursor` to trigger the guard. The test runs fast despite 101 mock responses.
- The `super::MAX_PAGES` reference works cleanly from the `tests` submodule to access the module-level constant.


### 2026-02-25: reconnect_server() lock hardening

- **Pattern**: `reconnect_server()` held `servers.write()` during transport spawn + connect + list_tools — blocking all reads (tool calls, list_servers, etc.) during potentially slow network I/O.
- **Fix**: Read lock briefly to grab config + Arc<old_state>, do all I/O outside any lock, then write lock only for validate + cap check + insert.
- **Key detail**: tool_cap check in reconnect must exclude the reconnecting server's own tools from `current_tool_count` (filter by name), unlike `add_server()` which counts all existing.
- **validate_tools() in reconnect**: The existing `validate_tools()` checks cross-server collision against `existing_servers` — since the old entry is still in the map during reconnect, it will see the server's own old tools. This is fine because the tool names from the new connection are compared against *other* servers' tools, and the old entry for this server is about to be replaced.
- **Testing reconnect validation**: Added `reconnect_server_with_client()` as a `#[cfg(test)]` helper that mirrors the real reconnect flow but accepts a pre-connected `McpClient`, avoiding the need to mock transport spawning. This parallels the existing `add_server_with_client()` pattern.
- **Error recovery**: On validation failure after reconnect I/O, the new client is closed but the old server entry remains in the map — the server stays registered with its previous tools rather than being removed.

### 2026-02-25: Transport defect fixes (stderr, graceful shutdown, SSE endpoint)

- `bytes::Bytes` is a transitive dep via `reqwest` but cannot be imported directly with `use bytes::Bytes` in Rust 2021 edition without adding `bytes` to `[dependencies]`. Workaround: map the stream to `Vec<u8>` in the caller before passing to `sse_listener`, avoiding the transitive type in the function signature.
- `StdioTransport` stderr pipe must be consumed to prevent OS pipe buffer deadlock. Spawning a background tokio task with `BufReader::read_line` loop and `tracing::debug!` is sufficient.
- Graceful shutdown pattern: close stdin → wait with timeout → kill. The 3s timeout via `tokio::time::timeout(Duration::from_secs(3), child.wait())` gives well-behaved servers a chance to exit cleanly.
- `closed: bool` flag in struct prevents double-kill: `close()` sets it, `Drop` checks it and returns early. This avoids redundant kill attempts when both `close()` and `Drop` run.
- SSE endpoint discovery via `oneshot::channel` in `new()` with 30s timeout ensures the POST endpoint is available before any `send()` call, eliminating the race condition where `send()` could fail because the endpoint event hadn't arrived yet.
- The `__sse_event`/`__sse_data` marker pattern in `receive()` was removed since endpoint is now resolved during construction. The oneshot sender is consumed (`Option::take()`) so it fires exactly once.
- Cargo.toml for zeroclaw-mcp had SSE deps (`reqwest`, `futures-util`) and `[features]` section that may be missing on disk due to concurrent edits — always verify file state before editing.

### 2026-02-25: Transport defect fixes (stderr drain + graceful shutdown) — retry

- **Scope discipline**: Previous attempt was reverted due to scope creep (SSE transport, new deps). This retry touches ONLY `transport.rs` with zero new dependencies.
- **No `closed` flag needed**: The `stdin.take()` / `stdout = None` / `stderr_task.take()` pattern already handles double-close safely. A `closed: bool` flag is unnecessary complexity.
- **stderr drain**: `child.stderr.take()` + `BufReader` + `tokio::spawn` loop with `read_line`. Logs via `tracing::debug!`. Task handle stored in struct for cleanup.
- **Graceful shutdown order**: close stdin → close stdout → abort stderr task → `tokio::time::timeout(3s, child.wait())` → if timeout: kill → wait.
- **Drop cleanup**: abort stderr task first, then `start_kill()` if process still running. Both `close()` and `Drop` use `.take()` on the stderr task handle so abort is idempotent.
- **Pre-existing compile errors**: `registry.rs` has missing `validate_server_name` method — prevents `cargo test` from compiling. Not related to transport changes. `cargo clippy` passes clean.
- **Import additions**: `use std::time::Duration;` and `warn` added to tracing import. No new crate dependencies.

### 2026-02-25: Pagination defect fix (client.rs only)
- **Clean implementation**: All three list methods (`list_tools`, `list_resources`, `list_prompts`) now loop with `nextCursor` handling and `MAX_PAGES` guard.
- **No downcast needed in tests**: Verifying pagination via result correctness (2 tools from 2 pages) is sufficient — no need to inspect sent requests via transport downcast.
- **`super::MAX_PAGES`** reference from test submodule works cleanly for the max-pages guard test.
- **MAX_PAGES test queues exactly 100 responses** (not 101) — the loop runs MAX_PAGES iterations, each with `nextCursor`, then breaks. The guard fires when cursor is still `Some` after the loop.
- **No transport.rs changes needed**: The `as_any()` pattern doesn't exist on `McpTransport` trait, and adding it would violate the single-file constraint. Result-based verification is the right approach.
- **cargo test (51 tests) + cargo clippy both pass clean** on first attempt.

### 2026-02-25: Registry defect fixes (Mutex wrapping, TOCTOU, validation)

- **Mutex wrapping**: `McpServerState.client` wrapped in `tokio::sync::Mutex<McpClient>` to allow read-lock on `servers` RwLock while still getting `&mut McpClient` for calls. This is the right choice since all McpClient methods are async.
- **Read vs Write lock**: `call_tool()`, `get_all_resources()`, `get_all_prompts()` now use `self.servers.read().await` + `state.client.lock().await` instead of `self.servers.write().await`. This allows concurrent reads (list_servers, get_all_tools) during potentially slow tool calls.
- **TOCTOU fix**: Both `add_server()` and `add_server_with_client()` had a read-lock for cap check followed by a separate write-lock for insert. Merged into a single write-lock section to prevent concurrent `add_server()` calls from exceeding the cap.
- **Server name validation**: Added `validate_server_name()` that rejects empty/whitespace-only names. Called from both `add_server()` and `add_server_with_client()`.
- **Tool name character validation**: Added to `validate_tools()` — checks `is_empty()` and `chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')`. No regex dependency needed.
- **remove_server Mutex**: Updated to use `.client.lock().await.close().await` chain since client is now behind Mutex.
- **Test compatibility**: Existing tests worked without changes since they use `add_server_with_client()` which handles the Mutex wrapping internally. Added 4 new tests: empty name, whitespace name, invalid tool chars, empty tool name.
- **Key insight**: Using `char`-level validation instead of regex avoids adding a dependency and is more idiomatic Rust for simple patterns.### 2026-02-25: MCP system prompt sanitization

- **sanitize_mcp_text()** placed as a module-level private fn before , since it's not a method — it's a pure utility.
- **regex crate** already in Cargo.toml deps (v1.10), so  inside the fn body works with no new deps.
- **6 injection points**: server name (x2 loops), res.uri, res.description, prompt.name, prompt.description — all now sanitized with 256 char max.
- **Pre-existing errors**: 5 errors in service/gateway modules (FeishuConfig, EstopConfig, OtpConfig, handle_wati_verify, enable_linger_if_needed) — none from agent.rs.

### 2026-02-26: Generation counter, client close fix, code fence wrapping

- **Fix A (validate_tools close)**: `add_server_with_client()` was missing the close-on-failure pattern that `add_server()` already had at lines 92-95. Wrapped `validate_tools` call in `if let Err(e)` with `client.close().await` before returning error.
- **Fix B1 (generation counter)**: Added `AtomicU64` field to `McpRegistry` with `SeqCst` ordering. Incremented after `servers.insert()` in both `add_server()` and `add_server_with_client()`, and after `servers.remove()` in `remove_server()`.
- **Fix B2 (Agent caching)**: Added `mcp_generation: u64` (init `u64::MAX` to force first fetch) and `mcp_cached_context: String` to Agent struct. Resources/prompts only re-fetched when `registry.generation()` differs from cached value.
- **Fix C (code fences)**: Wrapped MCP context in triple-backtick fences inside the cached string. Updated stripping logic to detect both new fenced format and legacy unfenced format for backward compat.
- **Brace alignment gotcha**: When replacing a multi-line block inside nested `if let` + `if` blocks, easy to lose a closing brace for the outer block.
- **Pre-existing warnings**: 4 unused import warnings from hooks/mod.rs and tools/mod.rs — known and unrelated.
