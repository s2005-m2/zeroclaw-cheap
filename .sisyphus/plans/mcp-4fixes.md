# MCP 4-Defect Fixes — C1, C3, H1, H3

## TL;DR

> **Quick Summary**: Fix 4 verified defects in ZeroClaw's MCP subsystem: system prompt accumulation bug, process leak on validation failure, Drop panic, and lock-held-during-IPC.
> 
> **Deliverables**:
> - Patched `agent.rs`: strip old MCP context before re-injecting each turn
> - Patched `registry.rs`: close client on validate_tools failure + release read lock before IPC
> - Patched `transport.rs`: safe Drop impl without unwrap
> 
> **Estimated Effort**: Short
> **Parallel Execution**: YES — 2 waves
> **Critical Path**: Task 1-4 (all independent) → Task 5 (verify)

---

## Context

### Original Request
Fix 4 specific MCP defects identified in analysis: C1 (system prompt accumulation), C3 (process leak), H1 (Drop panic), H3 (lock during IPC). Other issues are by-design — user wants ZeroClaw to self-manage MCP with full autonomy.

### Research Findings
- All source files read and verified
- Oracle consultation confirmed all 4 defects
- Existing `.sisyphus/plans/mcp-defect-fixes.md` is stale (references non-existent code)

---

## Work Objectives

### Core Objective
Eliminate 4 correctness/stability defects in MCP subsystem with minimal surgical patches.

### Concrete Deliverables
- `src/agent/agent.rs`: MCP context stripped before re-injection each turn
- `crates/zeroclaw-mcp/src/registry.rs`: client closed on validate_tools failure + IPC outside read lock
- `crates/zeroclaw-mcp/src/transport.rs`: Drop uses safe unwrap alternative

### Definition of Done
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes (all existing + any new tests)

### Must Have
- C1 fix: system prompt no longer accumulates MCP context across turns
- C3 fix: client.close() called when validate_tools fails in add_server
- H1 fix: Drop impl cannot panic
- H3 fix: read lock released before IPC in call_tool, get_all_resources, get_all_prompts

### Must NOT Have (Guardrails)
- No architectural changes (no new traits, modules, crates)
- No new dependencies
- No public API signature changes
- No changes to files outside the 3 target files
- Do NOT touch mcp_manage.rs (remove without autonomy check is by-design)
- Do NOT add timeouts (separate concern)
- Do NOT add generation counter / caching (separate concern)

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
- **Infrastructure exists**: YES (cargo test, MockTransport)
- **Automated tests**: YES (Tests-after for C3)
- **Framework**: `cargo test`

### QA Policy
Every task includes agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{slug}.txt`.

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (All independent — 4 parallel):
├── Task 1: agent.rs — fix MCP context accumulation [quick]
├── Task 2: registry.rs — close client on validate_tools failure [quick]
├── Task 3: transport.rs — safe Drop impl [quick]
├── Task 4: registry.rs — release read lock before IPC [deep]

Wave 2 (Verification):
├── Task 5: Full cargo fmt + clippy + test [quick]

Wave FINAL (After ALL tasks — 4 parallel):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
├── Task F4: Scope fidelity check (deep)

Critical Path: Tasks 1-4 → Task 5 → F1-F4
Max Concurrent: 4 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | — | 5 |
| 2 | — | 5 |
| 3 | — | 5 |
| 4 | — | 5 |
| 5 | 1-4 | F1-F4 |

### Agent Dispatch Summary

- **Wave 1**: T1 → `quick`, T2 → `quick`, T3 → `quick`, T4 → `deep`
- **Wave 2**: T5 → `quick`
- **FINAL**: F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

> Implementation + Test = ONE Task. Never separate.
> EVERY task MUST have: Recommended Agent Profile + Parallelization info + QA Scenarios.

- [x] 1. **Fix MCP context accumulation in system prompt**

  **What to do**:
  - In `agent.rs` `turn()`, before injecting MCP resources/prompts into the system message, strip any previous MCP context first
  - Find the system message's content, look for `\n[MCP Resources]\n` or `\n[MCP Prompts]\n` sentinel, truncate content at that position
  - Then append the fresh MCP context as before
  - This prevents N copies of MCP context accumulating over N turns
  - The logic change is: move the `if let Some(ConversationMessage::Chat(sys_msg))` block to wrap the entire resources/prompts section, and add truncation before the conditional append

  **Exact change** (lines 522-551 of agent.rs):
  Replace the current block with:
  ```rust
  // Inject MCP resources and prompts into system prompt
  let resources = registry.get_all_resources().await;
  let prompts = registry.get_all_prompts().await;
  if let Some(ConversationMessage::Chat(sys_msg)) = self.history.first_mut() {
      if sys_msg.role == "system" {
          // Strip previous MCP context to prevent accumulation across turns
          if let Some(start) = sys_msg.content.find("\n[MCP Resources]\n")
              .or_else(|| sys_msg.content.find("\n[MCP Prompts]\n"))
          {
              sys_msg.content.truncate(start);
          }
          if !resources.is_empty() || !prompts.is_empty() {
              let mut mcp_context = String::new();
              if !resources.is_empty() {
                  mcp_context.push_str("\n[MCP Resources]\n");
                  for (server, res) in &resources {
                      let desc = sanitize_mcp_text(res.description.as_deref().unwrap_or(""), 256);
                      let server_s = sanitize_mcp_text(server, 256);
                      let uri_s = sanitize_mcp_text(&res.uri, 256);
                      mcp_context.push_str(&format!("  {}: {} \u2014 {}\n", server_s, uri_s, desc));
                  }
              }
              if !prompts.is_empty() {
                  mcp_context.push_str("\n[MCP Prompts]\n");
                  for (server, prompt) in &prompts {
                      let desc = sanitize_mcp_text(prompt.description.as_deref().unwrap_or(""), 256);
                      let server_s = sanitize_mcp_text(server, 256);
                      let name_s = sanitize_mcp_text(&prompt.name, 256);
                      mcp_context
                          .push_str(&format!("  {}: {} \u2014 {}\n", server_s, name_s, desc));
                  }
              }
              sys_msg.content.push_str(&mcp_context);
          }
      }
  }
  ```

  **Must NOT do**:
  - Do not change the tool refresh logic (lines 504-520)
  - Do not change sanitize_mcp_text()
  - Do not change the system prompt builder

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single block replacement in one file, clear before/after
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4)
  - **Blocks**: Task 5
  - **Blocked By**: None

  **References**:
  - `src/agent/agent.rs:522-551` — Current MCP context injection block (the target)
  - `src/agent/agent.rs:229-242` — `sanitize_mcp_text()` function (do not modify, just use)
  - `src/agent/agent.rs:502-520` — MCP tool refresh block (do not modify)

  **WHY Each Reference Matters**:
  - Lines 522-551 are the exact block to replace — executor must understand the before/after
  - sanitize_mcp_text is called within the block and must be preserved
  - Tool refresh block is adjacent and must not be touched

  **Acceptance Criteria**:
  - [ ] System message content does NOT grow across multiple turn() calls when MCP context is unchanged
  - [ ] MCP resources/prompts still appear in system message when present
  - [ ] When resources/prompts become empty, old MCP context is stripped
  - [ ] `cargo check --bin zeroclaw` compiles

  **QA Scenarios:**
  ```
  Scenario: Agent compiles with fix
    Tool: Bash
    Steps:
      1. Run `cargo check --bin zeroclaw`
    Expected Result: Compiles without errors
    Evidence: .sisyphus/evidence/task-1-compile.txt
  ```

  **Commit**: YES
  - Message: `fix(mcp): strip old MCP context before re-injection in agent turn`
  - Files: `src/agent/agent.rs`
  - Pre-commit: `cargo check --bin zeroclaw`


- [x] 2. **Close client on validate_tools failure in add_server()**

  **What to do**:
  - In `registry.rs` `add_server()`, line 92: `self.validate_tools(&tools, &server_name).await?;`
  - The `?` returns immediately on error, but `client` (which holds a spawned child process) is never closed
  - Replace the bare `?` with explicit error handling that closes the client before returning:
  ```rust
  if let Err(e) = self.validate_tools(&tools, &server_name).await {
      let _ = client.close().await;
      return Err(e);
  }
  ```
  - This matches the pattern already used for tool_cap check at line 96-97

  **Must NOT do**:
  - Do not change validate_tools() itself
  - Do not change the tool_cap check (it already closes correctly)
  - Do not change add_server_with_client()

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single 3-line replacement in one method
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3, 4)
  - **Blocks**: Task 5
  - **Blocked By**: None

  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:92` — The bare `?` that leaks the client
  - `crates/zeroclaw-mcp/src/registry.rs:96-97` — The tool_cap check pattern to follow (closes client before bail)
  - `crates/zeroclaw-mcp/src/registry.rs:66-124` — Full `add_server()` method for context

  **WHY Each Reference Matters**:
  - Line 92 is the exact defect location — executor must see the bare `?`
  - Lines 96-97 show the correct pattern already in use 4 lines below

  **Acceptance Criteria**:
  - [ ] When validate_tools fails, client.close() is called before returning error
  - [ ] Existing `test_builtin_collision_rejected` still passes
  - [ ] Existing `test_tool_cap_enforced` still passes
  - [ ] `cargo test --package zeroclaw-mcp` passes

  **QA Scenarios:**
  ```
  Scenario: Builtin collision closes client cleanly
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_builtin_collision_rejected`
    Expected Result: Test passes
    Evidence: .sisyphus/evidence/task-2-validate-close.txt
  ```

  **Commit**: YES
  - Message: `fix(mcp): close client on validate_tools failure to prevent process leak`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`
- [x] 3. **Safe Drop impl to prevent panic in StdioTransport**

  **What to do**:
  - In `transport.rs` Drop impl, line 242: `self.child.try_wait().unwrap().is_some()`
  - `try_wait()` returns `io::Result<Option<ExitStatus>>` — the `unwrap()` panics if `Err`
  - Panic in Drop causes process abort (double-fault during stack unwinding)
  - Replace with safe alternative:
  ```rust
  impl Drop for StdioTransport {
      fn drop(&mut self) {
          if let Some(task) = self.stderr_task.take() {
              task.abort();
          }
          match self.child.try_wait() {
              Ok(Some(_)) => { /* Process already exited */ }
              _ => { let _ = self.child.start_kill(); }
          }
      }
  }
  ```

  **Must NOT do**:
  - Do not change close() method
  - Do not change stderr task handling (already correct)
  - Do not add async operations in Drop

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 1-line change in Drop impl
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4)
  - **Blocks**: Task 5
  - **Blocked By**: None

  **References**:
  - `crates/zeroclaw-mcp/src/transport.rs:235-248` — Drop impl to fix
  - `crates/zeroclaw-mcp/src/transport.rs:184-232` — close() method (do not touch, reference only)

  **WHY Each Reference Matters**:
  - Line 242 is the exact defect — `unwrap()` on `try_wait()` in Drop
  - close() shows the graceful shutdown pattern for comparison

  **Acceptance Criteria**:
  - [ ] Drop impl has no `unwrap()` calls
  - [ ] Process is still killed if not already exited
  - [ ] `cargo test --package zeroclaw-mcp test_stdio_transport` passes

  **QA Scenarios:**
  ```
  Scenario: Transport tests pass with safe Drop
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp test_stdio_transport`
    Expected Result: All stdio transport tests pass
    Evidence: .sisyphus/evidence/task-3-safe-drop.txt
  ```

  **Commit**: YES
  - Message: `fix(mcp): safe Drop impl to prevent panic on process cleanup`
  - Files: `crates/zeroclaw-mcp/src/transport.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`

- [x] 4. **Release read lock before IPC in registry call_tool/get_all_resources/get_all_prompts**

  **What to do**:
  - Three methods in `registry.rs` hold `servers.read().await` during network IPC, blocking all write operations
  - Fix each method to clone the needed reference inside the lock, drop the lock, then do IPC

  **4a. `call_tool()` (lines 279-302):**
  Replace with:
  ```rust
  pub async fn call_tool(
      &self,
      server_name: &str,
      tool_name: &str,
      args: Option<serde_json::Value>,
  ) -> Result<McpToolCallResult> {
      debug!("Calling MCP tool '{}' on server '{}'", tool_name, server_name);
      let client = {
          let servers = self.servers.read().await;
          let server = servers
              .get(server_name)
              .with_context(|| format!("MCP server '{}' not found", server_name))?;
          Arc::clone(&server.client)
      }; // read lock released here
      let mut client = client.lock().await;
      client.call_tool(tool_name, args).await.with_context(|| {
          format!("Failed to call tool '{}' on server '{}'", tool_name, server_name)
      })
  }
  ```
  **Requires**: Change `McpServerState.client` from `tokio::sync::Mutex<McpClient>` to `Arc<tokio::sync::Mutex<McpClient>>`
  Update the struct definition at line 20 and all construction sites (lines 111, 156 in add_server/add_server_with_client).
  **4b. `get_all_resources()` (lines 305-326):**
  Replace with:
  ```rust
  pub async fn get_all_resources(&self) -> Vec<(String, McpResource)> {
      let server_clients: Vec<(String, Arc<tokio::sync::Mutex<McpClient>>)> = {
          let servers = self.servers.read().await;
          servers.iter().map(|(name, state)| {
              (name.clone(), Arc::clone(&state.client))
          }).collect()
      }; // read lock released
      let mut all_resources = Vec::new();
      for (server_name, client) in server_clients {
          let mut client = client.lock().await;
          match client.list_resources().await {
              Ok(resources) => {
                  for resource in resources {
                      all_resources.push((server_name.clone(), resource));
                  }
              }
              Err(e) => {
                  tracing::warn!("Failed to list resources from MCP server '{}': {}", server_name, e);
              }
          }
      }
      all_resources
  }
  ```
  **4c. `get_all_prompts()` (lines 328-350):** Same pattern as 4b but for prompts.
  **Must NOT do**:
  - Do not change validate_tools() or add_server() logic (except the Mutex→Arc<Mutex> type change)
  - Do not change the tool validation flow
  - Do not add timeouts (separate concern)
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Concurrency-sensitive refactor across multiple methods, requires understanding lock semantics
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3)
  - **Blocks**: Task 5
  - **Blocked By**: None
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:19-24` — `McpServerState` struct (change client field type)
  - `crates/zeroclaw-mcp/src/registry.rs:279-302` — `call_tool()` to refactor
  - `crates/zeroclaw-mcp/src/registry.rs:305-326` — `get_all_resources()` to refactor
  - `crates/zeroclaw-mcp/src/registry.rs:328-350` — `get_all_prompts()` to refactor
  - `crates/zeroclaw-mcp/src/registry.rs:108-115` — `add_server()` insert site (update Mutex→Arc<Mutex>)
  - `crates/zeroclaw-mcp/src/registry.rs:153-160` — `add_server_with_client()` insert site (same update)
  **WHY Each Reference Matters**:
  - Lines 19-24: struct field type must change to Arc<Mutex> to allow cloning out of read lock
  - Lines 279-350: the three methods that hold read lock during IPC — the core defect
  - Lines 108-115, 153-160: construction sites that must wrap client in Arc
  **Acceptance Criteria**:
  - [ ] `call_tool()` releases read lock before IPC
  - [ ] `get_all_resources()` releases read lock before IPC
  - [ ] `get_all_prompts()` releases read lock before IPC
  - [ ] `McpServerState.client` is `Arc<tokio::sync::Mutex<McpClient>>`
  - [ ] All existing registry tests pass
  - [ ] `cargo test --package zeroclaw-mcp` passes
  **QA Scenarios:**
  ```
  Scenario: Registry tests pass with lock refactor
    Tool: Bash
    Steps:
      1. Run `cargo test --package zeroclaw-mcp`
    Expected Result: All tests pass
    Evidence: .sisyphus/evidence/task-4-lock-refactor.txt
  ```
  **Commit**: YES
  - Message: `fix(mcp): release read lock before IPC in registry methods`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
  - Pre-commit: `cargo test --package zeroclaw-mcp`
- [x] 5. **Full cargo fmt + clippy + test verification**
  **What to do**:
  - Run `cargo fmt --all -- --check` and fix any formatting issues from Tasks 1-4
  - Run `cargo clippy --all-targets -- -D warnings` and fix any warnings
  - Run `cargo test` and verify all tests pass
  - Run `cargo test --package zeroclaw-mcp` specifically
  **Must NOT do**:
  - Do not add new features or refactors
  - Do not modify test expectations to make them pass
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Verification only
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (sequential, after all implementation)
  - **Blocks**: F1-F4
  - **Blocked By**: Tasks 1-4
  **References**:
  - All modified files from Tasks 1-4
  **Acceptance Criteria**:
  - [ ] `cargo fmt --all -- --check` passes
  - [ ] `cargo clippy --all-targets -- -D warnings` passes
  - [ ] `cargo test` passes with 0 failures
  **QA Scenarios:**
  ```
  Scenario: Full CI check
    Tool: Bash
    Steps:
      1. Run `cargo fmt --all -- --check`
      2. Run `cargo clippy --all-targets -- -D warnings`
      3. Run `cargo test`
    Expected Result: All three pass with zero errors/warnings
    Evidence: .sisyphus/evidence/task-5-full-ci.txt
  ```
  **Commit**: NO (verification only)

## Final Verification Wave

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists. For each "Must NOT Have": search codebase for forbidden patterns. Check evidence files exist.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review changed files for unwrap in non-test code, empty catches, unused imports.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | VERDICT`

- [x] F3. **Real Manual QA** — `unspecified-high`
  Run `cargo test` end-to-end. Verify each new test exists and passes. Check no existing tests removed.
  Output: `Tests [N/N pass] | New Tests [N added] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 match. Check "Must NOT do" compliance. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Wave 1 commit 1**: `fix(mcp): strip old MCP context before re-injection in agent turn` — agent.rs
- **Wave 1 commit 2**: `fix(mcp): close client on validate_tools failure` — registry.rs
- **Wave 1 commit 3**: `fix(mcp): safe Drop impl to prevent panic` — transport.rs
- **Wave 1 commit 4**: `fix(mcp): release read lock before IPC in registry` — registry.rs

---

## Success Criteria

### Verification Commands
```bash
cargo fmt --all -- --check   # Expected: clean
cargo clippy --all-targets -- -D warnings  # Expected: no warnings
cargo test                   # Expected: all pass
cargo test --package zeroclaw-mcp  # Expected: all MCP tests pass
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] No new dependencies
- [ ] No public API changes
