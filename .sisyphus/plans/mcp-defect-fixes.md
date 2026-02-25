# MCP Defect Fixes — Security, Correctness & Robustness

## TL;DR

> **Quick Summary**: Fix 15 identified defects in ZeroClaw's MCP implementation spanning security bypasses, resource leaks, deadlocks, and missing validation. All fixes are minimal and surgical — no architectural changes.
> 
> **Deliverables**:
> - Hardened `reconnect_server()` with tool validation and lock-minimized I/O
> - Deadlock-proof stdio transport (stderr consumption)
> - Sanitized MCP context injection into system prompt
> - Pagination guards, tool name validation, resource caching
> - Graceful shutdown, reconnect_locks cleanup, SSE endpoint handling
> 
> **Estimated Effort**: Medium
> **Parallel Execution**: YES — 4 waves
> **Critical Path**: Task 1 → Task 3 → Task 8 → Task 12 → Final

---

## Context

### Original Request
Fix all MCP defects identified in the analysis except Windows atomic write compatibility. Minimal implementation, no new defects.

### Interview Summary
**Key Discussions**:
- 15 defects identified across Critical/High/Medium/Low severity
- User wants精简实现 (minimal, surgical fixes)
- Windows `persist_config` atomic write excluded from scope

**Research Findings**:
- MCP crate: `crates/zeroclaw-mcp/src/` (client.rs, registry.rs, transport.rs, types.rs, config.rs, jsonrpc.rs, lib.rs)
- Integration: `src/tools/mcp_bridge.rs`, `src/tools/mcp_manage.rs`, `src/agent/agent.rs`
- Config: `src/config/schema.rs` (McpConfig struct)
- Existing tests: MockTransport-based unit tests in client.rs and registry.rs
- Stack: Rust, tokio, async_trait, anyhow, serde_json, tracing

### Metis Review
**Identified Gaps** (addressed):
- reconnect_server lock restructure must preserve the per-server reconnect mutex semantics
- Pagination MAX_PAGES must not break legitimate large tool sets
- Sanitization must not corrupt valid Unicode in MCP descriptions
- Generation-cached resources/prompts must still refresh on registry changes

---

## Work Objectives

### Core Objective
Eliminate all identified security, correctness, and robustness defects in the MCP subsystem without architectural changes.

### Concrete Deliverables
- Patched `registry.rs`: reconnect validation, lock restructure, tool name validation, reconnect_locks cleanup, generation fix
- Patched `client.rs`: pagination guards
- Patched `transport.rs`: stderr handling, graceful shutdown, SSE endpoint wait
- Patched `agent.rs`: sanitized MCP context injection, generation-cached resources/prompts
- Updated tests covering each fix

### Definition of Done
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes (all existing + new tests)
- [ ] No new `unsafe`, `unwrap()` in non-test code, or `#[allow(...)]` added

### Must Have
- All 15 fixes (minus Windows atomic write) implemented
- Each fix has at least one test proving the defect is resolved
- Zero regressions in existing tests

### Must NOT Have (Guardrails)
- No architectural changes (no new traits, no new modules, no new crates)
- No new dependencies added to Cargo.toml
- No changes to public API signatures (only internal behavior)
- No speculative "future-proof" abstractions
- No formatting-only changes mixed with functional changes
- No changes to files outside MCP scope (except `src/agent/agent.rs` for integration fixes)
- Do NOT touch `src/config/schema.rs` — config shape stays the same
- Do NOT add health check mechanism (design improvement, out of scope)
- Do NOT implement `roots/list` (interop improvement, out of scope)
- Do NOT change `McpContent::to_display_string()` for images (cosmetic, out of scope)

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (cargo test, MockTransport in client.rs and registry.rs)
- **Automated tests**: YES (Tests-after — add focused tests per fix)
- **Framework**: `cargo test` (standard Rust test harness)

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **All tasks**: Use Bash — `cargo test`, `cargo clippy`, `cargo fmt --check`
- **Specific scenarios**: targeted `cargo test <test_name>` for each new test

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — independent fixes in separate files):
├── Task 1: registry.rs — reconnect validation + lock restructure [deep]
├── Task 2: transport.rs — stderr handling [quick]
├── Task 3: client.rs — pagination MAX_PAGES guard [quick]
├── Task 4: transport.rs (SSE) — endpoint discovery fix [quick]
├── Task 10: transport.rs — graceful shutdown + closed flag [quick]

Wave 2 (Registry hardening — depends on Task 1):
├── Task 5: registry.rs — tool name validation in validate_tools() [quick]
├── Task 6: registry.rs — reconnect_locks cleanup + add_server_with_client fixes [quick]
├── Task 7: registry.rs — close client on validate_tools failure [quick]

Wave 3 (Integration — independent of registry changes):
├── Task 8: agent.rs — sanitize MCP context injection [quick]
├── Task 9: agent.rs — cache resources/prompts behind generation counter [quick]

Wave 4 (Verification):
├── Task 11: Full test suite + clippy + fmt verification [quick]

Wave FINAL (After ALL tasks — independent review, 4 parallel):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
├── Task F4: Scope fidelity check (deep)

Critical Path: Task 1 → Task 5 → Task 11 → F1-F4
Parallel Speedup: ~60% faster than sequential
Max Concurrent: 5 (Wave 1)

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | — | 5, 6, 7 |
| 2 | — | 11 |
| 3 | — | 11 |
| 4 | — | 11 |
| 5 | 1 | 11 |
| 6 | 1 | 11 |
| 7 | 1 | 11 |
| 8 | — | 11 |
| 9 | — | 11 |
| 10 | — | 11 |
| 11 | 1-10 | F1-F4 |

### Agent Dispatch Summary

- **Wave 1**: 4 tasks — T1 → `deep`, T2 → `quick`, T3 → `quick`, T4 → `quick`
- **Wave 2**: 3 tasks — T5 → `quick`, T6 → `quick`, T7 → `quick`
- **Wave 3**: 2 tasks — T8 → `quick`, T9 → `quick`
- **Wave 4**: 1 task — T10 → `quick`
- **FINAL**: 4 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

> Implementation + Test = ONE Task. Never separate.
> EVERY task MUST have: Recommended Agent Profile + Parallelization info + QA Scenarios.


- [ ] 1. **Harden `reconnect_server()` — tool validation + lock restructure**

  **What to do**:
  - Move transport spawn + `McpClient::connect()` + `list_tools()` OUTSIDE the `servers.write()` lock
  - Only acquire write lock for the final atomic swap (close old + insert new)
  - After `list_tools()`, call `self.validate_tools(&tools, server_name, &servers)` (requires brief read lock)
  - Check tool_cap after validation, bail if exceeded
  - Keep the per-server reconnect mutex (`reconnect_locks`) to prevent duplicate spawns
  - Sequence: acquire reconnect lock → spawn transport outside servers lock → connect → list_tools → acquire write lock → close old → validate → swap → release

  **Must NOT do**:
  - Do not change `add_server()` logic in this task (separate task)
  - Do not remove the per-server reconnect lock
  - Do not change the `call_tool()` retry logic

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Concurrency-sensitive lock restructuring requires careful reasoning
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4)
  - **Blocks**: Tasks 5, 6, 7
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `crates/zeroclaw-mcp/src/registry.rs:381-462` — Current `reconnect_server()` implementation to restructure
  - `crates/zeroclaw-mcp/src/registry.rs:71-150` — `add_server()` pattern showing correct validate+insert flow
  - `crates/zeroclaw-mcp/src/registry.rs:200-232` — `validate_tools()` method to reuse

  **API/Type References**:
  - `crates/zeroclaw-mcp/src/registry.rs:20-24` — `McpServerState` struct (client, tools, config)
  - `crates/zeroclaw-mcp/src/registry.rs:27-38` — `McpRegistry` fields (servers, tool_cap, reconnect_locks, generation)

  **WHY Each Reference Matters**:
  - `reconnect_server()` is the target function — executor must understand current lock ordering
  - `add_server()` shows the correct pattern: spawn → connect → list → validate → insert under lock
  - `validate_tools()` is the validation function that reconnect currently skips

  **Acceptance Criteria**:
  - [ ] `reconnect_server()` calls `validate_tools()` after `list_tools()`
  - [ ] `reconnect_server()` checks tool_cap after validation
  - [ ] Network I/O (spawn, connect, list_tools) happens outside `servers.write()` lock
  - [ ] Write lock is only held for close-old + validate + insert
  - [ ] Existing `test_call_tool` and `test_add_server` tests still pass

  **QA Scenarios:**

  ```
  Scenario: Reconnect validates tools (happy path)
    Tool: Bash
    Preconditions: cargo test compiles
    Steps:
      1. Run `cargo test --package zeroclaw-mcp reconnect`
      2. Verify test passes
    Expected Result: All reconnect-related tests pass
    Evidence: .sisyphus/evidence/task-1-reconnect-validate.txt

  Scenario: Reconnect rejects tool collision
    Tool: Bash
    Preconditions: New test added that mocks a server returning a builtin-colliding tool on reconnect
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_reconnect_rejects_builtin_collision`
      2. Verify test passes — reconnect should fail with collision error
    Expected Result: Test passes, reconnect returns error
    Evidence: .sisyphus/evidence/task-1-reconnect-reject-collision.txt
  ```

  **Commit**: YES
  - Message: `fix(mcp): validate tools after reconnect and minimize lock scope`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`

---

- [ ] 2. **Prevent stdio deadlock — consume or null stderr**

  **What to do**:
  - In `StdioTransport::new_with_policy()`, spawn a background tokio task that reads stderr and logs lines via `tracing::debug!`
  - Alternatively (simpler): change `.stderr(std::process::Stdio::piped())` to `.stderr(std::process::Stdio::null())` — this is the minimal fix
  - Recommended: use the stderr-consuming task approach so MCP server errors are visible in debug logs
  - Take stderr from child, spawn `tokio::spawn(async move { BufReader lines loop, tracing::debug each line })`, store JoinHandle
  - In `close()`, abort the stderr task
  - In `Drop`, abort the stderr task (non-async, use `JoinHandle::abort()`)

  **Must NOT do**:
  - Do not change the stdout reading logic
  - Do not change the stdin writing logic
  - Do not add new public API methods

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single-file change, clear pattern
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3, 4)
  - **Blocks**: Task 10
  - **Blocked By**: None

  **References**:
  - `crates/zeroclaw-mcp/src/transport.rs:120-127` — `StdioTransport` struct fields to extend
  - `crates/zeroclaw-mcp/src/transport.rs:144-203` — `new_with_policy()` where stderr is piped but never consumed
  - `crates/zeroclaw-mcp/src/transport.rs:186` — `.stderr(std::process::Stdio::piped())` line to change
  - `crates/zeroclaw-mcp/src/transport.rs:336-368` — `close()` method to update
  - `crates/zeroclaw-mcp/src/transport.rs:371-392` — `Drop` impl to update

  **WHY Each Reference Matters**:
  - Line 186 is the root cause — stderr is piped but never read
  - close() and Drop need to abort the stderr task to prevent leaks

  **Acceptance Criteria**:
  - [ ] stderr is consumed (task) or nulled
  - [ ] `close()` cleans up stderr task
  - [ ] `Drop` cleans up stderr task
  - [ ] Existing transport tests still pass

  **QA Scenarios:**
  ```
  Scenario: Transport spawns and closes cleanly
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_stdio_transport`
      2. Verify all stdio transport tests pass
    Expected Result: All pass, no hangs
    Evidence: .sisyphus/evidence/task-2-stderr-fix.txt
  ```

  **Commit**: YES
  - Message: `fix(mcp): consume stderr to prevent child process deadlock`
  - Files: `crates/zeroclaw-mcp/src/transport.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`

- [ ] 3. **Add pagination guard to prevent infinite loops**

  **What to do**:
  - Add `const MAX_PAGES: usize = 100;` in `client.rs` near existing constants
  - In `list_tools()`, `list_resources()`, `list_prompts()`: add a page counter, bail if `pages > MAX_PAGES`
  - Error message: `"MCP server returned too many pages ({MAX_PAGES}), possible infinite pagination"`
  - Add test: mock transport that always returns nextCursor, verify client bails after MAX_PAGES

  **Must NOT do**:
  - Do not change `call_tool()` or `read_resource()` or `get_prompt()` (not paginated)
  - Do not make MAX_PAGES configurable (YAGNI)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Repetitive pattern across 3 methods, simple counter
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4)
  - **Blocks**: Task 10
  - **Blocked By**: None

  **References**:
  - `crates/zeroclaw-mcp/src/client.rs:92-121` — `list_tools()` pagination loop
  - `crates/zeroclaw-mcp/src/client.rs:179-207` — `list_resources()` pagination loop
  - `crates/zeroclaw-mcp/src/client.rs:255-283` — `list_prompts()` pagination loop
  - `crates/zeroclaw-mcp/src/client.rs:128-129` — existing MAX_CONTENT_ITEMS/MAX_TEXT_BYTES constants as pattern

  **WHY Each Reference Matters**:
  - All three pagination loops have identical structure — add counter to each
  - Existing constants show the naming/placement convention

  **Acceptance Criteria**:
  - [ ] `list_tools()` bails after MAX_PAGES
  - [ ] `list_resources()` bails after MAX_PAGES
  - [ ] `list_prompts()` bails after MAX_PAGES
  - [ ] Test proves infinite pagination is caught

  **QA Scenarios:**
  ```
  Scenario: Pagination guard triggers
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_list_tools_pagination_guard`
      2. Verify test passes — client should bail with pagination error
    Expected Result: Test passes, error contains "too many pages"
    Evidence: .sisyphus/evidence/task-3-pagination-guard.txt
  ```

  **Commit**: YES
  - Message: `fix(mcp): add pagination guard to prevent infinite loops`
  - Files: `crates/zeroclaw-mcp/src/client.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`

- [ ] 4. **SSE transport — handle endpoint discovery before send**
  **What to do**:
  - In `SseTransport::new()`, after spawning the SSE listener task, wait (with timeout) for the first `endpoint` event before returning
  - Add a `tokio::sync::oneshot` channel: listener sends endpoint URL through it, `new()` awaits it with `tokio::time::timeout(Duration::from_secs(30), rx)`
  - Once endpoint is received, set `self.post_endpoint` immediately in `new()`
  - Remove the `__sse_event`/`__sse_data` marker handling from `receive()` — endpoint is already known
  - If timeout expires, bail with clear error: `"MCP SSE server did not send endpoint event within 30s"`
  **Must NOT do**:
  - Do not change stdio transport
  - Do not change the SSE listener's message forwarding logic (only add oneshot for endpoint)
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Scoped to SSE module behind feature flag
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3)
  - **Blocks**: Task 10
  - **Blocked By**: None
  **References**:
  - `crates/zeroclaw-mcp/src/transport.rs:437-471` — `SseTransport::new()` to modify
  - `crates/zeroclaw-mcp/src/transport.rs:484-545` — `sse_listener()` to add oneshot send
  - `crates/zeroclaw-mcp/src/transport.rs:550-568` — `send()` that currently fails if no endpoint
  - `crates/zeroclaw-mcp/src/transport.rs:586-631` — `receive()` with `__sse_event` handling to simplify
  **WHY Each Reference Matters**:
  - `new()` is where endpoint discovery must happen before returning a usable transport
  - `sse_listener()` needs to send endpoint via oneshot before forwarding to mpsc
  - `send()` and `receive()` become simpler once endpoint is guaranteed at construction
  **Acceptance Criteria**:
  - [ ] `SseTransport::new()` blocks until endpoint is discovered or times out
  - [ ] `send()` no longer needs to check for missing endpoint (it's always set)
  - [ ] Timeout produces clear error message
  - [ ] Existing SSE-related code compiles with `--features sse`
  **QA Scenarios:**
  ```
  Scenario: SSE code compiles
    Tool: Bash
    Steps:
      1. Run `cargo check --package zeroclaw-mcp --features sse` (if sse feature exists)
      2. If sse feature doesn't exist in test env, run `cargo check --package zeroclaw-mcp`
    Expected Result: Compiles without errors
    Evidence: .sisyphus/evidence/task-4-sse-compile.txt
  ```
  **Commit**: YES
  - Message: `fix(mcp): resolve SSE endpoint during transport construction`
  - Files: `crates/zeroclaw-mcp/src/transport.rs`
  - Pre-commit: `cargo check --package zeroclaw-mcp`
- [ ] 5. **Validate tool names in `validate_tools()`**
  **What to do**:
  - In `validate_tools()`, add character validation for each tool name: `[a-zA-Z0-9_-]`, max 128 chars, non-empty
  - Reuse the same character set as `validate_server_name()` but with higher length limit (tool names can be longer)
  - Extract a shared helper `fn validate_name(name: &str, label: &str, max_len: usize) -> Result<()>` used by both
  - Bail with: `"MCP server '' tool '{}' contains invalid characters"` on violation
  **Must NOT do**:
  - Do not change `validate_server_name()` behavior (only extract shared logic)
  - Do not validate tool descriptions or schemas
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Small addition to existing validation function
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6, 7)
  - **Blocks**: Task 10
  - **Blocked By**: Task 1 (registry.rs changes)
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:200-232` — `validate_tools()` to extend
  - `crates/zeroclaw-mcp/src/registry.rs:549-565` — `validate_server_name()` pattern to reuse
  **Acceptance Criteria**:
  - [ ] Tool names with spaces/newlines/special chars are rejected
  - [ ] Empty tool names are rejected
  - [ ] Tool names > 128 chars are rejected
  - [ ] Valid tool names still pass
  - [ ] Test proves invalid tool name is caught
  **QA Scenarios:**
  ```
  Scenario: Invalid tool name rejected
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_tool_name_validation`
    Expected Result: Test passes, invalid names rejected with clear error
    Evidence: .sisyphus/evidence/task-5-tool-name-validation.txt
  ```
  **Commit**: YES (groups with Tasks 6, 7)
  - Message: `fix(mcp): validate tool names and harden registry`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`
- [ ] 6. **Cleanup reconnect_locks + fix `add_server_with_client()`**
  **What to do**:
  - In `remove_server()`: after removing from `servers` map, also remove from `self.reconnect_locks`
  - In `add_server_with_client()`: add `Self::validate_server_name(&name)?;` at the start
  - In `add_server_with_client()`: add `self.generation.fetch_add(1, Ordering::SeqCst);` after successful insert
  **Must NOT do**:
  - Do not change `add_server()` or `reconnect_server()` (handled in Task 1)
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Three one-liner additions
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 7)
  - **Blocks**: Task 10
  - **Blocked By**: Task 1
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:244-263` — `remove_server()` to add reconnect_locks cleanup
  - `crates/zeroclaw-mcp/src/registry.rs:153-198` — `add_server_with_client()` to add validation + generation
  - `crates/zeroclaw-mcp/src/registry.rs:35-36` — `reconnect_locks` field
  **Acceptance Criteria**:
  - [ ] `remove_server()` cleans up reconnect_locks entry
  - [ ] `add_server_with_client()` validates server name
  - [ ] `add_server_with_client()` increments generation
  - [ ] Existing tests pass
  **QA Scenarios:**
  ```
  Scenario: remove_server cleans reconnect_locks
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_remove_server`
    Expected Result: Test passes
    Evidence: .sisyphus/evidence/task-6-cleanup.txt
  ```
  **Commit**: YES (groups with Task 5)
  - Message: `fix(mcp): validate tool names and harden registry`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`
- [ ] 7. **Close client on `validate_tools()` failure in `add_server()`**
  **What to do**:
  - In `add_server()`, after `validate_tools()` fails (the bail path), add `let _ = client.close().await;` before the bail
  - Restructure: wrap the validation in a block, if it errors, close client then return the error
  - Pattern: `if let Err(e) = self.validate_tools(...) { let _ = client.close().await; return Err(e); }`
  **Must NOT do**:
  - Do not change the tool_cap check (it already closes client)
  - Do not change `add_server_with_client()` validation (handled in Task 6)
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single error-path fix
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 6)
  - **Blocks**: Task 10
  - **Blocked By**: Task 1
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:71-150` — `add_server()` method
  - `crates/zeroclaw-mcp/src/registry.rs:118` — `self.validate_tools()` call that can bail without closing
  **Acceptance Criteria**:
  - [ ] Client is closed when `validate_tools()` fails
  - [ ] Existing tests pass
  **QA Scenarios:**
  ```
  Scenario: Builtin collision closes client
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_builtin_collision_rejected`
    Expected Result: Test passes (existing test, verifies no regression)
    Evidence: .sisyphus/evidence/task-7-close-on-fail.txt
  ```
  **Commit**: YES (groups with Tasks 5, 6)
  - Message: `fix(mcp): validate tool names and harden registry`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`
- [ ] 8. **Sanitize MCP context injection into system prompt**
  **What to do**:
  - In `agent.rs` `turn()`, where MCP resources/prompts are injected into system prompt (~line 547-578):
  - Wrap all MCP-provided strings in a sanitizer that replaces control characters and trims length
  - Add helper: `fn sanitize_mcp_string(s: &str, max_len: usize) -> String` — strips `\n`, `\r`, `\t`, truncates to max_len, replaces non-printable chars
  - Apply to: `res.uri`, `res.description`, `prompt.name`, `prompt.description`
  - Wrap the entire MCP context block in triple-backtick code fence to prevent LLM interpretation:
    ```
    mcp_context.push_str("```\n");
    // ... resource/prompt lines ...
    mcp_context.push_str("```\n");
    ```
  - Max lengths: uri=512, description=256, name=128
  **Must NOT do**:
  - Do not change the sentinel-based truncation logic (it works correctly)
  - Do not change how tools are injected (only resources/prompts context)
  - Do not modify the system prompt builder
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: String sanitization, single file
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Task 9)
  - **Blocks**: Task 10
  - **Blocked By**: None
  **References**:
  - `src/agent/agent.rs:547-578` — MCP context injection block to modify
  - `src/agent/agent.rs:553` — MCP_SENTINEL constant (keep as-is)
  - `src/agent/agent.rs:557-560` — Resource formatting (sanitize uri + description)
  - `src/agent/agent.rs:564-567` — Prompt formatting (sanitize name + description)
  **WHY Each Reference Matters**:
  - Lines 557-567 are where unsanitized MCP strings enter the system prompt
  - The code fence wrapping prevents LLM from interpreting injected content as instructions
  **Acceptance Criteria**:
  - [ ] MCP strings are sanitized (control chars stripped, length limited)
  - [ ] MCP context block is wrapped in code fence
  - [ ] Existing agent compilation succeeds
  **QA Scenarios:**
  ```
  Scenario: Agent compiles with sanitization
    Tool: Bash
    Steps:
      1. Run `cargo check --bin zeroclaw`
    Expected Result: Compiles without errors
    Evidence: .sisyphus/evidence/task-8-sanitize-compile.txt
  ```
  **Commit**: YES (groups with Task 9)
  - Message: `fix(mcp): sanitize MCP context injection and cache resources`
  - Files: `src/agent/agent.rs`
  - Pre-commit: `cargo check --bin zeroclaw`
- [ ] 9. **Cache resources/prompts behind generation counter**
  **What to do**:
  - In `agent.rs`, add two new fields to Agent struct: `mcp_cached_resources: Vec<(String, McpResource)>` and `mcp_cached_prompts: Vec<(String, McpPrompt)>`
  - In `turn()`, move the `get_all_resources()` / `get_all_prompts()` calls inside the `if current_gen != self.mcp_generation` block (same block that refreshes tools)
  - Cache the results in the new fields, use cached values for system prompt injection
  - This eliminates N*2 IPC calls per turn when nothing changed
  **Must NOT do**:
  - Do not change the MCP context injection format (handled in Task 8)
  - Do not change the generation counter logic
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Move existing code into conditional block + add cache fields
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Task 8)
  - **Blocks**: Task 10
  - **Blocked By**: None
  **References**:
  - `src/agent/agent.rs:527-546` — generation check block where tools are refreshed
  - `src/agent/agent.rs:548-549` — `get_all_resources()` / `get_all_prompts()` calls to move inside generation block
  - `src/agent/agent.rs:39-41` — Agent struct fields (mcp_registry, mcp_generation) to extend
  **WHY Each Reference Matters**:
  - The generation block already handles tool caching — resources/prompts should follow the same pattern
  - Agent struct needs new cache fields
  **Acceptance Criteria**:
  - [ ] Resources/prompts only fetched when generation changes
  - [ ] Cached values used for system prompt injection
  - [ ] Agent compiles and existing tests pass
  **QA Scenarios:**
  ```
  Scenario: Agent compiles with caching
    Tool: Bash
    Steps:
      1. Run `cargo check --bin zeroclaw`
    Expected Result: Compiles without errors
    Evidence: .sisyphus/evidence/task-9-cache-compile.txt
  ```
  **Commit**: YES (groups with Task 8)
  - Message: `fix(mcp): sanitize MCP context injection and cache resources`
  - Files: `src/agent/agent.rs`
  - Pre-commit: `cargo check --bin zeroclaw`
- [ ] 10. **Graceful shutdown for StdioTransport**
  **What to do**:
  - In `StdioTransport::close()`, after closing stdin, add a short wait before kill:
    `match tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await { Ok(_) => return Ok(()), Err(_) => { /* proceed to kill */ } }`
  - Only call `self.child.kill()` if the timeout expires (server didn't exit gracefully)
  - Add a `closed: bool` flag to `StdioTransport` struct
  - Set `closed = true` in `close()`
  - In `Drop`, skip kill if `closed` is true (prevents double-kill warnings)
  **Must NOT do**:
  - Do not change the stdin/stdout handling
  - Do not add new public API
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Small lifecycle improvement in single method
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3, 4) — can be done alongside Task 2 since they touch different methods
  - **Blocks**: Task 11
  - **Blocked By**: None
  **References**:
  - `crates/zeroclaw-mcp/src/transport.rs:336-368` — `close()` method to add graceful wait
  - `crates/zeroclaw-mcp/src/transport.rs:371-392` — `Drop` impl to add `closed` guard
  - `crates/zeroclaw-mcp/src/transport.rs:120-127` — struct fields to add `closed: bool`
  **Acceptance Criteria**:
  - [ ] `close()` waits up to 3s for graceful exit before kill
  - [ ] `Drop` skips kill if `close()` was already called
  - [ ] Existing transport tests pass
  **QA Scenarios:**
  ```
  Scenario: Transport close is graceful
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_stdio_transport_close_kills_process`
    Expected Result: Test passes
    Evidence: .sisyphus/evidence/task-10-graceful-shutdown.txt
  ```
  **Commit**: YES (groups with Task 2)
  - Message: `fix(mcp): consume stderr to prevent child process deadlock`
  - Files: `crates/zeroclaw-mcp/src/transport.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`
- [ ] 11. **Full test suite + clippy + fmt verification**
  **What to do**:
  - Run `cargo fmt --all -- --check` and fix any formatting issues
  - Run `cargo clippy --all-targets -- -D warnings` and fix any warnings
  - Run `cargo test` and verify all tests pass (existing + new)
  - Run `cargo test --package zeroclaw-mcp` specifically for MCP crate
  - Verify no new `unwrap()` in non-test code, no `#[allow(...)]` added
  **Must NOT do**:
  - Do not add new features or refactors
  - Do not modify test expectations to make them pass (fix the code instead)
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Verification only, no implementation
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4 (sequential, after all implementation)
  - **Blocks**: F1-F4
  - **Blocked By**: Tasks 1-10
  **References**:
  - All modified files from Tasks 1-10
  **Acceptance Criteria**:
  - [ ] `cargo fmt --all -- --check` passes
  - [ ] `cargo clippy --all-targets -- -D warnings` passes
  - [ ] `cargo test` passes with 0 failures
  - [ ] No new `unwrap()` in non-test code
  **QA Scenarios:**
  ```
  Scenario: Full CI check
    Tool: Bash
    Steps:
      1. Run `cargo fmt --all -- --check`
      2. Run `cargo clippy --all-targets -- -D warnings`
      3. Run `cargo test`
    Expected Result: All three pass with zero errors/warnings
    Evidence: .sisyphus/evidence/task-11-full-ci.txt
  ```
  **Commit**: NO (verification only)
## Final Verification Wave

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review all changed files for: `as any`/`unwrap()` in non-test code, empty catches, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction, generic names.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Run `cargo test` end-to-end. Verify each new test exists and passes. Check that no existing tests were removed or modified (only additions). Run `cargo clippy` and verify zero warnings.
  Output: `Tests [N/N pass] | New Tests [N added] | Clippy [PASS/FAIL] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git diff). Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Detect cross-task contamination. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Wave 1**: `fix(mcp): harden reconnect with tool validation and lock restructure` — registry.rs
- **Wave 1**: `fix(mcp): prevent stdio deadlock by consuming stderr` — transport.rs
- **Wave 1**: `fix(mcp): add pagination guard to prevent infinite loops` — client.rs
- **Wave 1**: `fix(mcp): handle SSE endpoint discovery before send` — transport.rs
- **Wave 2**: `fix(mcp): validate tool names and cleanup reconnect locks` — registry.rs
- **Wave 3**: `fix(mcp): sanitize MCP context injection and cache resources` — agent.rs
- **Wave 4**: `chore(mcp): verify all fixes pass CI checks` — no files

---

## Success Criteria

### Verification Commands
```bash
cargo fmt --all -- --check   # Expected: no output (clean)
cargo clippy --all-targets -- -D warnings  # Expected: no warnings
cargo test                   # Expected: all pass, 0 failures
cargo test --package zeroclaw-mcp  # Expected: all MCP tests pass
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] No new dependencies
- [ ] No public API changes
