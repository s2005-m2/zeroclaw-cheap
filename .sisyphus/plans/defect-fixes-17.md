# ZeroClaw 17-Defect Fix Plan

## TL;DR

> **Quick Summary**: Fix 17 functional defects across 5 subsystems (MCP, Skills, Hooks, VPN, Feishu) identified in a comprehensive audit. All fixes are isolated, behavior-preserving patches — no architectural changes.
> 
> **Deliverables**:
> - 5 MCP fixes: error surfacing, .mcp.json persistence, timeout, duplicate rejection, close error handling
> - 4 Skills fixes: blank line analysis, dirty flag atomicity, clock rollback, load failure logging
> - 3 Hooks fixes: reload stamp preservation, before_tool_call consistency, priority docs
> - 2 VPN fixes: port polling, health check concurrency
> - 3 Feishu fixes: retry backoff, token TOCTOU, async file watcher
> 
> **Estimated Effort**: Medium-Large
> **Parallel Execution**: YES — 4 waves
> **Critical Path**: Wave 1 (independent fixes) → Wave 2 (dependent fixes) → Wave 3 (cross-module) → Wave FINAL (verification)

---

## Context

### Original Request
User requested a comprehensive audit of 5 subsystems (MCP, dynamic skills, dynamic hooks, VPN, Feishu). The audit produced 37 findings. User selected 17 to fix, explicitly marking the remaining 20 as acceptable architecture tradeoffs.

### Interview Summary
**Key Discussions**:
- #1: No auto-reconnect — surface MCP server crash errors directly to zeroclaw agent
- #5: Reject duplicate server names with error, tell agent to pick a new name
- #7: Close errors must be surfaced to zeroclaw agent via callback
- #12: SystemTime rollback → return false (treat as "not yet time to sync")
- #20: Add priority documentation in HOOK.toml example and manifest comments
- #9: Already fixed (git clone --depth 1 already present) — removed from scope

**Research Findings**:
- MCP spec: timeouts SHOULD be configurable, default 30s tool calls, 10s init. Send cancellation notification on timeout. Reject duplicate names.
- .mcp.json: standard format with `mcpServers` HashMap. `McpServerConfig` already derives `Serialize`. Atomic writes recommended.
- Token TOCTOU: double-checked locking with `tokio::sync::RwLock` is idiomatic Rust pattern
- File watcher: `tokio::sync::mpsc` with `blocking_send()` bridges notify callbacks to async
- Retry: use existing `src/providers/reliable.rs` pattern (exponential backoff, 2x, 10s cap) — do NOT add external retry crates
- Clock rollback: `Instant` is monotonic; for file metadata comparison, clamp `duration_since` errors

### Metis Review
**Identified Gaps** (addressed):
- #31 retry must follow existing `reliable.rs` pattern, not introduce `reqwest-retry` crate
- #2 scope is smaller than assumed — `McpServerConfig` already derives `Serialize`, only write path needed
- #18 confirmed: `before_tool_call` is the ONLY hook returning `Cancel` on error — genuine inconsistency
- #4 timeout: need cancellation notification per MCP spec, not just timeout wrapper
- #22 port polling: need max retry count to avoid infinite loop on misconfigured port
- #26 health check lock: use `tokio::sync::Mutex` (not semaphore) — simpler, sufficient

---

## Work Objectives

### Core Objective
Fix 17 isolated functional defects to improve reliability, consistency, and error observability across MCP, Skills, Hooks, VPN, and Feishu subsystems.

### Concrete Deliverables
- Patched files across 5 subsystems (see TODOs for exact file list)
- All fixes are behavior-preserving patches — no API changes, no new dependencies (except possibly `tempfile` for atomic writes)

### Definition of Done
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes (all existing tests)
- [ ] Each fix verified by agent-executed QA scenario

### Must Have
- Error messages surfaced to zeroclaw agent must be actionable (include server name, error type, suggested action)
- MCP timeout must be configurable (not hardcoded)
- Retry pattern must match existing `reliable.rs` style
- All fixes must be minimal — touch only the defect, not surrounding code

### Must NOT Have (Guardrails)
- NO new external crates for retry (use existing `reliable.rs` pattern)
- NO auto-reconnect for MCP servers (user explicitly rejected)
- NO changes to public trait signatures
- NO changes to config schema keys (backward compatibility)
- NO refactoring of surrounding code "while here"
- NO speculative "future-proof" abstractions
- NO changes to files outside the 17 fix scope

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (cargo test)
- **Automated tests**: Tests-after where applicable (not TDD — these are bug fixes in existing code)
- **Framework**: cargo test (Rust built-in)

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Rust code**: Use Bash — `cargo build`, `cargo test`, `cargo clippy`, grep for expected patterns
- **Config/docs**: Use Bash — validate file format, check content presence

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately — independent single-file fixes, MAX PARALLEL):
├── Task 1:  #13 Skill load failure logging (skills/mod.rs) [quick]
├── Task 2:  #12 SystemTime clock rollback (skills/mod.rs) [quick]
├── Task 3:  #11 Dirty flag atomicity (skills/mod.rs) [quick]
├── Task 4:  #10 Suspicious blank lines analysis (skills/mod.rs) [quick]
├── Task 5:  #20 Hook priority documentation (hooks/manifest.rs) [quick]
├── Task 6:  #18 before_tool_call → Continue (hooks/dynamic.rs) [quick]
├── Task 7:  #17 Hook reload stamp preservation (hooks/reload.rs) [quick]
├── Task 8:  #33 FileWatcher async mpsc (docs_sync/watcher.rs) [quick]
└── Task 9:  #5 Duplicate server name rejection (mcp/registry.rs) [quick]

Wave 2 (After Wave 1 — slightly more complex, still parallel):
├── Task 10: #1 MCP crash error surfacing (mcp/registry.rs, client.rs) [unspecified-high]
├── Task 11: #4 MCP tool call timeout (mcp/registry.rs, client.rs) [unspecified-high]
├── Task 12: #7 MCP close error surfacing (mcp/registry.rs) [unspecified-high]
├── Task 13: #22 VPN port polling (vpn/runtime.rs) [unspecified-high]
└── Task 14: #26 VPN health check concurrency (vpn/health.rs) [unspecified-high]

Wave 3 (After Wave 2 — cross-concern or multi-file):
├── Task 15: #2 MCP .mcp.json persistence (mcp/registry.rs, config.rs, mcp_manage.rs) [deep]
├── Task 16: #32 Feishu token TOCTOU (docs_sync/client.rs) [deep]
└── Task 17: #31 Feishu retry backoff (docs_sync/client.rs) [deep]

Wave FINAL (After ALL tasks — verification):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review — cargo fmt/clippy/test (unspecified-high)
├── Task F3: Behavioral QA — verify each fix (unspecified-high)
└── Task F4: Scope fidelity check (deep)

Critical Path: Wave 1 → Wave 2 (Tasks 10-12 depend on Task 9 for registry.rs) → Wave 3 → FINAL
Parallel Speedup: ~65% faster than sequential
Max Concurrent: 9 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1-4  | None      | —      | 1    |
| 5-8  | None      | —      | 1    |
| 9    | None      | 10,11,12,15 | 1 |
| 10   | 9         | 15     | 2    |
| 11   | 9         | 15     | 2    |
| 12   | 9         | 15     | 2    |
| 13   | None      | —      | 2    |
| 14   | None      | —      | 2    |
| 15   | 10,11,12  | —      | 3    |
| 16   | None      | 17     | 3    |
| 17   | 16        | —      | 3    |
| F1-F4| ALL       | —      | FINAL|

### Agent Dispatch Summary

- **Wave 1**: **9 tasks** — T1-T9 → `quick`
- **Wave 2**: **5 tasks** — T10-T12 → `unspecified-high`, T13-T14 → `unspecified-high`
- **Wave 3**: **3 tasks** — T15-T17 → `deep`
- **FINAL**: **4 tasks** — F1 → `oracle`, F2-F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

> Implementation + verification = ONE task. EVERY task has QA scenarios.

- [x] 1. **#13 — Skill load failure: add warning log on TOML parse error**

  **What to do**:
  - In `src/skills/mod.rs`, find the `if let Ok(skill) = load_skill_toml(...)` pattern around lines 201-208
  - Add an `else` / `Err(e)` branch that calls `tracing::warn!("Failed to load skill TOML from {}: {}", path.display(), e)`
  - Do NOT change the control flow — still skip the failed skill, just log it

  **Must NOT do**:
  - Do not change the Ok path behavior
  - Do not panic or bail on load failure
  - Do not add retry logic

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2-9)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `src/skills/mod.rs:201-208` — the `if let Ok(skill)` pattern that silently skips failures
  - `src/skills/mod.rs:1-20` — existing `use tracing::...` imports (confirm `warn` is imported)
  - `src/hooks/loader.rs` — similar pattern where hook load failures ARE logged (follow this pattern)

  **Acceptance Criteria**:
  - [ ] `cargo build` succeeds
  - [ ] `cargo clippy --all-targets -- -D warnings` passes
  - [ ] grep confirms `tracing::warn` or `warn!` exists in the Err branch near line 201-208

  **QA Scenarios:**
  ```
  Scenario: Warning log emitted on skill load failure
    Tool: Bash (grep)
    Steps:
      1. grep -n 'warn!' src/skills/mod.rs — find the new warning log
      2. Verify the warning message includes both the path and the error
      3. cargo build 2>&1 — confirm no compilation errors
      4. cargo clippy --all-targets -- -D warnings 2>&1 — confirm no warnings
    Expected Result: grep shows warn! with path and error context near load_skill_toml call
    Evidence: .sisyphus/evidence/task-1-skill-load-warn.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `fix(skills): log warning on skill TOML load failure (#13)`
  - Files: `src/skills/mod.rs`

- [x] 2. **#12 — SystemTime clock rollback: return false instead of true**

  **What to do**:
  - In `src/skills/mod.rs:460`, find `should_sync_open_skills()` function
  - Find the `SystemTime::now().duration_since(modified_at)` call
  - The current `else { return true }` branch triggers sync on every startup when clock rolls back
  - Change to `else { return false }` — treat clock rollback as "not yet time to sync"
  - Add a comment: `// Clock rollback detected — skip sync this cycle`

  **Must NOT do**:
  - Do not switch to `Instant` (file metadata uses `SystemTime`, can't avoid it)
  - Do not add complex clock drift detection

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3-9)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `src/skills/mod.rs:460` — `should_sync_open_skills()` function
  - `src/skills/mod.rs:455-475` — full function body with the `duration_since` call
  - Rust docs: `SystemTime::duration_since()` returns `Err` when `self` is earlier than `earlier`

  **Acceptance Criteria**:
  - [ ] The else branch returns `false` instead of `true`
  - [ ] Comment explains the rationale
  - [ ] `cargo build` succeeds

  **QA Scenarios:**
  ```
  Scenario: Clock rollback returns false
    Tool: Bash (grep)
    Steps:
      1. grep -A2 'duration_since' src/skills/mod.rs — find the duration_since call
      2. Verify the else/Err branch contains 'return false' or 'false'
      3. grep -i 'clock.*rollback\|rollback.*clock' src/skills/mod.rs — verify comment exists
      4. cargo build 2>&1 — confirm compilation
    Expected Result: else branch returns false with clock rollback comment
    Evidence: .sisyphus/evidence/task-2-clock-rollback.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `fix(skills): handle SystemTime rollback in open-skills sync (#12)`
  - Files: `src/skills/mod.rs`

- [x] 3. **#11 — Dirty flag atomicity: replace bool with AtomicBool**

  **What to do**:
  - In `src/skills/mod.rs`, find `SkillsState` struct (around line 51-54)
  - Change `dirty: bool` to `dirty: AtomicBool` (from `std::sync::atomic`)
  - Update all reads of `dirty` to use `.load(Ordering::Relaxed)`
  - Update all writes of `dirty` to use `.store(value, Ordering::Relaxed)`
  - `Relaxed` ordering is sufficient — dirty flag is advisory, not a synchronization primitive
  - If `SkillsState` derives `Clone`, implement `Clone` manually for the `AtomicBool` field

  **Must NOT do**:
  - Do not change the RwLock wrapping SkillsState
  - Do not add a separate Mutex for the dirty flag
  - Do not use `SeqCst` ordering (unnecessary overhead)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-2, 4-9)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `src/skills/mod.rs:51-54` — `SkillsState` struct definition with `dirty: bool`
  - `src/skills/mod.rs` — grep for `\.dirty` to find all read/write sites
  - `std::sync::atomic::AtomicBool` — Rust stdlib atomic bool
  - `src/skills/mod.rs:1-20` — check existing imports

  **Acceptance Criteria**:
  - [ ] `dirty` field is `AtomicBool` type
  - [ ] All `.dirty` accesses use atomic operations
  - [ ] `cargo build` succeeds
  - [ ] `cargo clippy --all-targets -- -D warnings` passes

  **QA Scenarios:**
  ```
  Scenario: AtomicBool correctly replaces bool
    Tool: Bash (grep + cargo)
    Steps:
      1. grep -n 'AtomicBool' src/skills/mod.rs — verify import and field type
      2. grep -n '\.dirty' src/skills/mod.rs — verify all accesses use .load() or .store()
      3. Confirm no raw `.dirty = true` or `.dirty = false` assignments remain
      4. cargo build 2>&1 — confirm compilation
      5. cargo test -p zeroclaw 2>&1 — confirm tests pass
    Expected Result: All dirty accesses are atomic, build and tests pass
    Evidence: .sisyphus/evidence/task-3-atomic-dirty.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `fix(skills): use AtomicBool for dirty flag to prevent data races (#11)`
  - Files: `src/skills/mod.rs`

- [x] 4. **#10 — Suspicious blank lines: analyze and fill if code was deleted**

  **What to do**:
  - In `src/skills/mod.rs`, examine lines 179-195 and 272-288 (17 blank lines each)
  - Use `git log -p src/skills/mod.rs` to check if code was removed from these locations
  - If code was deleted: determine what was there and whether it should be restored
  - If blank lines are just formatting artifacts: remove excess blank lines (keep max 1 between sections)
  - Document findings in a code comment if the gap is intentional

  **Must NOT do**:
  - Do not add speculative code — only restore if git history shows deletion
  - Do not reformat the entire file

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `['git-master']`
    - `git-master`: needed to investigate git history for deleted code

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-3, 5-9)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `src/skills/mod.rs:179-195` — first block of 17 blank lines (between security audit and SKILL.toml loading)
  - `src/skills/mod.rs:272-288` — second block of 17 blank lines
  - `src/skills/audit.rs` — security audit module (context for what precedes the first gap)

  **Acceptance Criteria**:
  - [ ] Git history analyzed for both blank line blocks
  - [ ] If code was deleted: restored or documented why not
  - [ ] If formatting artifact: excess blank lines removed (max 1 between sections)
  - [ ] `cargo build` succeeds

  **QA Scenarios:**
  ```
  Scenario: Blank lines analyzed and resolved
    Tool: Bash (git log + grep)
    Steps:
      1. git log --oneline -20 -- src/skills/mod.rs — find recent commits
      2. Check if blank line blocks still exist or were resolved
      3. cargo build 2>&1 — confirm compilation
    Expected Result: Blank lines either removed or documented with rationale
    Evidence: .sisyphus/evidence/task-4-blank-lines.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `fix(skills): clean up suspicious blank line blocks in mod.rs (#10)`
  - Files: `src/skills/mod.rs`

- [x] 5. **#20 — Hook priority documentation: add doc comments and HOOK.toml example**

  **What to do**:
  - In `src/hooks/manifest.rs`, find the `priority` field in `HookManifest` struct
  - Add a doc comment: `/// Hook execution priority. Higher number = runs first (descending order). Default: 0`
  - If there's a HOOK.toml example in the codebase or docs, add a comment showing priority usage:
    ```toml
    # priority: Higher number = runs first. Default: 0
    # Example: priority = 10 runs before priority = 5
    priority = 0
    ```

  **Must NOT do**:
  - Do not change priority sorting logic
  - Do not add priority validation

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-4, 6-9)
  - **Blocks**: None
  - **Blocked By**: None

  **References**:
  - `src/hooks/manifest.rs` — `HookManifest` struct, `priority` field
  - `src/hooks/runner.rs` — where hooks are sorted by priority (confirms descending order)
  - Any `HOOK.toml` example files in `docs/` or skill directories

  **Acceptance Criteria**:
  - [ ] Doc comment on `priority` field explains semantics
  - [ ] HOOK.toml example (if exists) has priority comment
  - [ ] `cargo build` succeeds (doc comments don't break compilation)

  **QA Scenarios:**
  ```
  Scenario: Priority documentation present
    Tool: Bash (grep)
    Steps:
      1. grep -B2 -A2 'priority' src/hooks/manifest.rs — verify doc comment exists
      2. grep -r 'priority.*Higher\|Higher.*priority' src/hooks/ — confirm documentation text
      3. cargo build 2>&1 — confirm compilation
    Expected Result: Doc comment explains "Higher number = runs first (descending order)"
    Evidence: .sisyphus/evidence/task-5-priority-docs.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `docs(hooks): document hook priority semantics in manifest (#20)`
  - Files: `src/hooks/manifest.rs`

- [x] 6. **#18 — before_tool_call failure: return Continue instead of Cancel**
  **What to do**:
  - In `src/hooks/dynamic.rs`, find the `before_tool_call` method around lines 332-338
  - Find the `Err(e)` branch that currently returns `HookResult::Cancel(...)`
  - Change to `HookResult::Continue((name, args))` — matching all other modifying hooks' error behavior
  - Keep the existing `warn!` log line
  **Must NOT do**:
  - Do not change any other hook method's error handling
  - Do not change the Ok path
  - Do not remove the warning log
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-5, 7-9)
  - **Blocks**: None
  - **Blocked By**: None
  **References**:
  - `src/hooks/dynamic.rs:332-338` — the `before_tool_call` Err branch returning Cancel
  - `src/hooks/dynamic.rs:280-300` — `before_model_resolve` Err branch returning Continue (pattern to follow)
  - `src/hooks/dynamic.rs:305-325` — `before_prompt_build` Err branch returning Continue (pattern to follow)
  - `src/hooks/traits.rs` — `HookResult` enum definition (Continue vs Cancel variants)
  **Acceptance Criteria**:
  - [ ] `before_tool_call` Err branch returns `HookResult::Continue((name, args))`
  - [ ] Warning log is preserved
  - [ ] All other hook methods unchanged
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: before_tool_call returns Continue on error
    Tool: Bash (grep)
    Steps:
      1. grep -A5 'before_tool_call' src/hooks/dynamic.rs | grep -i 'cancel\|continue'
      2. Verify no 'Cancel' in before_tool_call error path
      3. Verify 'Continue' is returned with (name, args)
      4. cargo build 2>&1 — confirm compilation
    Expected Result: before_tool_call Err branch returns Continue, not Cancel
    Evidence: .sisyphus/evidence/task-6-hook-continue.txt
  ```
  **Commit**: YES (groups with Wave 1)
  - Message: `fix(hooks): return Continue on before_tool_call failure for consistency (#18)`
  - Files: `src/hooks/dynamic.rs`
- [x] 7. **#17 — Hook reload timeout: preserve stamp file on timeout**
  **What to do**:
  - In `src/hooks/reload.rs`, find the reload logic around lines 75-97
  - Currently the stamp file is ALWAYS deleted at line 94 after reload attempt
  - Change logic: only delete stamp if reload SUCCEEDED
  - If reload timed out or failed, preserve the stamp so next cycle retries
  - Pattern: move stamp deletion inside the success branch, not after the match
  **Must NOT do**:
  - Do not change the reload timeout duration
  - Do not add retry logic inside the reload function itself
  - Do not change stamp file format or location
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-6, 8-9)
  - **Blocks**: None
  - **Blocked By**: None
  **References**:
  - `src/hooks/reload.rs:75-97` — reload logic with stamp deletion at line 94
  - `src/hooks/reload.rs:1-30` — stamp file path and creation logic
  **Acceptance Criteria**:
  - [ ] Stamp file only deleted on successful reload
  - [ ] Stamp preserved on timeout/failure (enables retry on next cycle)
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Stamp preserved on reload failure
    Tool: Bash (grep + read)
    Steps:
      1. Read src/hooks/reload.rs lines 75-100
      2. Verify stamp deletion (fs::remove_file or similar) is inside success branch only
      3. Verify timeout/error branches do NOT delete the stamp
      4. cargo build 2>&1 — confirm compilation
    Expected Result: Stamp deletion is conditional on success, not unconditional
    Evidence: .sisyphus/evidence/task-7-stamp-preserve.txt
  ```
  **Commit**: YES (groups with Wave 1)
  - Message: `fix(hooks): preserve reload stamp on timeout to enable retry (#17)`
  - Files: `src/hooks/reload.rs`
- [x] 8. **#33 — FileWatcher: replace std::sync::mpsc with tokio::sync::mpsc**
  **What to do**:
  - In `src/docs_sync/watcher.rs`, find the `std::sync::mpsc::channel` usage (around line 28)
  - Replace with `tokio::sync::mpsc::channel::<PathBuf>(100)` (bounded, capacity 100)
  - In the notify callback, use `tx.blocking_send(path)` instead of `tx.send(path)`
  - Update the `rx` field type from `std::sync::mpsc::Receiver<PathBuf>` to `tokio::sync::mpsc::Receiver<PathBuf>`
  - Update any consumer code that calls `rx.recv()` to use `rx.recv().await`
  - `blocking_send()` is correct here: it blocks the notify callback thread (sync), not the async runtime
  **Must NOT do**:
  - Do not change the notify watcher configuration
  - Do not add unbounded channel (use bounded with capacity 100)
  - Do not change the watched paths or event filtering logic
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-7, 9)
  - **Blocks**: None
  - **Blocked By**: None
  **References**:
  - `src/docs_sync/watcher.rs` (79 lines) — full file, the FileWatcher struct and watch() method
  - `src/docs_sync/sync.rs` — consumer of the watcher's rx channel (must update to async recv)
  - `src/docs_sync/mod.rs` — module wiring
  - Codebase already uses `tokio::sync::mpsc` in 16+ locations (e.g., `src/agent.rs:842`)
  **Acceptance Criteria**:
  - [ ] `std::sync::mpsc` replaced with `tokio::sync::mpsc`
  - [ ] `blocking_send()` used in notify callback
  - [ ] Consumer uses `.recv().await`
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Async mpsc channel in FileWatcher
    Tool: Bash (grep)
    Steps:
      1. grep -n 'std::sync::mpsc' src/docs_sync/watcher.rs — should return NO matches
      2. grep -n 'tokio::sync::mpsc' src/docs_sync/watcher.rs — should return matches
      3. grep -n 'blocking_send' src/docs_sync/watcher.rs — verify blocking_send in callback
      4. cargo build 2>&1 — confirm compilation
    Expected Result: No std::sync::mpsc, uses tokio::sync::mpsc with blocking_send
    Evidence: .sisyphus/evidence/task-8-async-watcher.txt
  ```
  **Commit**: YES (groups with Wave 1)
  - Message: `fix(docs_sync): use tokio::sync::mpsc in FileWatcher for async compatibility (#33)`
  - Files: `src/docs_sync/watcher.rs`, `src/docs_sync/sync.rs`
- [x] 9. **#5 — Duplicate MCP server name: reject with error**
  **What to do**:
  - In `crates/zeroclaw-mcp/src/registry.rs`, find `add_server` method (around line 105-126)
  - Before `servers.insert()`, check if `servers.contains_key(&server_name)`
  - If duplicate, return error: `bail!("MCP server '' already exists. Please use a different name or remove the existing server first.", server_name)`
  - The error message must be actionable for the zeroclaw agent
  **Must NOT do**:
  - Do not add an "update" or "replace" action (out of scope)
  - Do not silently overwrite
  - Do not change remove_server behavior
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1-8)
  - **Blocks**: Tasks 10, 11, 12, 15 (they also touch registry.rs)
  - **Blocked By**: None
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:105-126` — `add_server` method with `servers.insert()`
  - `crates/zeroclaw-mcp/src/registry.rs:85-103` — server validation logic before add
  **Acceptance Criteria**:
  - [ ] Duplicate name check before insert
  - [ ] Error message includes server name and actionable guidance
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Duplicate server name rejected
    Tool: Bash (grep)
    Steps:
      1. grep -n 'contains_key\|already exists' crates/zeroclaw-mcp/src/registry.rs
      2. Verify duplicate check exists before insert
      3. Verify error message includes server name
      4. cargo build 2>&1 — confirm compilation
    Expected Result: contains_key check before insert, error message with server name
    Evidence: .sisyphus/evidence/task-9-duplicate-reject.txt
  ```
  **Commit**: YES (groups with Wave 1)
  - Message: `fix(mcp): reject duplicate server names with actionable error (#5)`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`
- [x] 10. **#1 — MCP server crash: surface error to zeroclaw agent**
  **What to do**:
  - In `crates/zeroclaw-mcp/src/registry.rs`, find `call_tool` method
  - When transport IO fails (server crash/disconnect), the error currently propagates as an opaque IO error
  - Wrap the error with actionable context: `"MCP server '{}' is unavailable ({}). The server may have crashed. Remove and re-add it to reconnect.", server_name, original_error`
  - In `crates/zeroclaw-mcp/src/client.rs`, find `call_tool` method
  - Add similar error context wrapping for transport-level failures
  - Do NOT add auto-reconnect — user explicitly rejected this
  **Must NOT do**:
  - No auto-reconnect or retry logic
  - No background health monitoring
  - Do not change the transport layer itself
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 11-14)
  - **Blocks**: Task 15
  - **Blocked By**: Task 9 (shares registry.rs)
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs` — `call_tool` method, error propagation paths
  - `crates/zeroclaw-mcp/src/client.rs` — `call_tool` method, transport IO error handling
  - `crates/zeroclaw-mcp/src/transport.rs` — `StdioTransport` error types (context for what errors look like)
  - MCP spec: "Implementations SHOULD be prepared to handle protocol version mismatch, failure to negotiate capabilities, request timeouts"
  **Acceptance Criteria**:
  - [ ] Transport errors include server name in message
  - [ ] Error message suggests actionable next step (remove and re-add)
  - [ ] No auto-reconnect logic added
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Error message includes server name and action
    Tool: Bash (grep)
    Steps:
      1. grep -n 'unavailable\|crashed\|reconnect' crates/zeroclaw-mcp/src/registry.rs
      2. grep -n 'unavailable\|crashed\|reconnect' crates/zeroclaw-mcp/src/client.rs
      3. Verify error messages include server name placeholder and actionable guidance
      4. cargo build 2>&1 — confirm compilation
    Expected Result: Error messages with server name and "remove and re-add" guidance
    Evidence: .sisyphus/evidence/task-10-crash-error.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `fix(mcp): surface server crash errors with actionable context (#1)`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`, `crates/zeroclaw-mcp/src/client.rs`
- [x] 11. **#4 — MCP tool call timeout: wrap call_tool in tokio::time::timeout**
  **What to do**:
  - In `crates/zeroclaw-mcp/src/registry.rs`, find `call_tool` method
  - Wrap the `client.call_tool()` call in `tokio::time::timeout(duration, ...)`
  - Default timeout: 30 seconds (per MCP spec recommendation)
  - Make timeout configurable via `McpServerConfig` or a constant
  - On timeout, send MCP cancellation notification per spec: `notifications/cancelled`
  - Error message: `"MCP tool call '{}' on server '{}' timed out after {:?}. The server may be unresponsive.", tool_name, server_name, duration`
  - In `crates/zeroclaw-mcp/src/client.rs`, add a `send_cancel_notification` helper if needed
  **Must NOT do**:
  - Do not hardcode timeout without making it configurable
  - Do not add retry on timeout (just fail with clear error)
  - Do not change the tool call request format
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 10, 12-14)
  - **Blocks**: Task 15
  - **Blocked By**: Task 9 (shares registry.rs)
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs` — `call_tool` method
  - `crates/zeroclaw-mcp/src/client.rs` — `call_tool` method, transport send/receive
  - `crates/zeroclaw-mcp/src/jsonrpc.rs` — JSON-RPC message types (for cancellation notification)
  - MCP spec: "SDKs SHOULD allow timeouts to be configured on a per-request basis"
  - MCP spec: "sender SHOULD issue a cancellation notification and stop waiting"
  - `tokio::time::timeout` — Rust async timeout wrapper
  **Acceptance Criteria**:
  - [ ] `call_tool` wrapped in `tokio::time::timeout`
  - [ ] Default timeout is 30 seconds
  - [ ] Timeout is configurable (constant or config field)
  - [ ] Timeout error message includes tool name, server name, and duration
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Timeout wrapper present on call_tool
    Tool: Bash (grep)
    Steps:
      1. grep -n 'timeout' crates/zeroclaw-mcp/src/registry.rs — verify timeout wrapper
      2. grep -n 'timed out' crates/zeroclaw-mcp/src/registry.rs — verify error message
      3. grep -n '30' crates/zeroclaw-mcp/src/registry.rs — verify default timeout value
      4. cargo build 2>&1 — confirm compilation
    Expected Result: timeout wrapper with 30s default, actionable error message
    Evidence: .sisyphus/evidence/task-11-timeout.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `fix(mcp): add configurable timeout for tool calls with cancellation (#4)`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`, `crates/zeroclaw-mcp/src/client.rs`
- [x] 12. **#7 — MCP close error: surface to zeroclaw agent**
  **What to do**:
  - In `crates/zeroclaw-mcp/src/registry.rs`, find `remove_server` method (around line 246-252)
  - Currently close errors propagate, but in validation paths (lines 101, 108, 152, 158) `let _ = client.close()` silently swallows errors
  - For `remove_server`: wrap close errors with context: `"Failed to cleanly close MCP server '{}': {}. Server removed from registry but process may still be running.", server_name, e`
  - For validation paths: at minimum add `tracing::warn!` on close failure instead of `let _ =`
  **Must NOT do**:
  - Do not retry close operations
  - Do not block removal on close failure (remove from registry regardless)
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 10-11, 13-14)
  - **Blocks**: Task 15
  - **Blocked By**: Task 9 (shares registry.rs)
  **References**:
  - `crates/zeroclaw-mcp/src/registry.rs:246-252` — `remove_server` method
  - `crates/zeroclaw-mcp/src/registry.rs:101,108,152,158` — `let _ = client.close()` silent swallows
  - `crates/zeroclaw-mcp/src/client.rs` — `close()` method implementation
  **Acceptance Criteria**:
  - [ ] `remove_server` close errors include server name and actionable message
  - [ ] Validation path close errors logged with `tracing::warn!`
  - [ ] No `let _ = client.close()` remains without at least a warning log
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Close errors logged or surfaced
    Tool: Bash (grep)
    Steps:
      1. grep -n 'let _ = .*close' crates/zeroclaw-mcp/src/registry.rs — should return NO matches
      2. grep -n 'warn!.*close\|close.*warn' crates/zeroclaw-mcp/src/registry.rs — verify warnings added
      3. grep -n 'still be running\|Failed to.*close' crates/zeroclaw-mcp/src/registry.rs — verify error context
      4. cargo build 2>&1 — confirm compilation
    Expected Result: No silent close swallows, all close errors logged or surfaced
    Evidence: .sisyphus/evidence/task-12-close-error.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `fix(mcp): surface close errors to agent and log in validation paths (#7)`
  - Files: `crates/zeroclaw-mcp/src/registry.rs`

- [x] 13. **#22 — VPN Clash startup: port polling instead of fixed sleep**
  **What to do**:
  - In `src/vpn/runtime.rs:222`, find the `tokio::time::sleep(Duration::from_millis(500))` after Clash process spawn
  - Replace with a polling loop that tries to connect to `127.0.0.1:{socks_port}`
  - Pattern: loop with `tokio::net::TcpStream::connect()`, sleep 100ms between attempts, max 20 attempts (2s total)
  - On success: break and continue
  - On max retries exceeded: return error `"Clash proxy failed to start: port {} not responding after 2s", socks_port`
  **Must NOT do**:
  - Do not use infinite loop (must have max retry count)
  - Do not change the Clash process spawn logic
  - Do not change the socks port configuration
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 10-12, 14)
  - **Blocks**: None
  - **Blocked By**: None
  **References**:
  - `src/vpn/runtime.rs:222` — the `sleep(500ms)` line after Clash spawn
  - `src/vpn/runtime.rs:200-240` — full startup sequence for context
  - `tokio::net::TcpStream::connect` — async TCP connection attempt
  **Acceptance Criteria**:
  - [ ] Fixed sleep replaced with port polling loop
  - [ ] Max retry count prevents infinite loop (20 attempts, 100ms each)
  - [ ] Clear error on timeout with port number
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Port polling replaces fixed sleep
    Tool: Bash (grep)
    Steps:
      1. grep -n 'sleep.*500\|from_millis(500)' src/vpn/runtime.rs — should return NO matches
      2. grep -n 'TcpStream::connect\|connect' src/vpn/runtime.rs — verify polling logic
      3. grep -n 'not responding\|failed to start' src/vpn/runtime.rs — verify error message
      4. cargo build 2>&1 — confirm compilation
    Expected Result: No fixed 500ms sleep, replaced with TcpStream polling loop
    Evidence: .sisyphus/evidence/task-13-port-polling.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `fix(vpn): replace fixed sleep with port polling for Clash startup (#22)`
  - Files: `src/vpn/runtime.rs`
- [x] 14. **#26 — VPN health check: add concurrency lock**
  **What to do**:
  - In `src/vpn/health.rs`, find `check_all_via_clash` method (around lines 162-195)
  - Add a `tokio::sync::Mutex` to prevent concurrent health checks from interfering
  - Pattern: add a `health_check_lock: Arc<tokio::sync::Mutex<()>>` field to the health checker struct
  - At the start of `check_all_via_clash`, acquire the lock: `let _guard = self.health_check_lock.lock().await`
  - If lock is already held, the second caller waits (not skips) — this prevents concurrent node switching
  **Must NOT do**:
  - Do not use `try_lock` (waiting is correct behavior here)
  - Do not use a semaphore (Mutex is simpler and sufficient)
  - Do not change the health check logic itself
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 10-13)
  - **Blocks**: None
  - **Blocked By**: None
  **References**:
  - `src/vpn/health.rs:162-195` — `check_all_via_clash` method
  - `src/vpn/health.rs:1-30` — struct definition (add lock field here)
  - `src/vpn/mod.rs` — where health checker is constructed (initialize lock)
  - `tokio::sync::Mutex` — async mutex for concurrency control
  **Acceptance Criteria**:
  - [ ] `tokio::sync::Mutex` field added to health checker
  - [ ] Lock acquired at start of `check_all_via_clash`
  - [ ] Lock initialized in constructor
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Concurrency lock on health check
    Tool: Bash (grep)
    Steps:
      1. grep -n 'Mutex' src/vpn/health.rs — verify Mutex import and field
      2. grep -n 'lock().await' src/vpn/health.rs — verify lock acquisition
      3. cargo build 2>&1 — confirm compilation
    Expected Result: Mutex field and lock acquisition in check_all_via_clash
    Evidence: .sisyphus/evidence/task-14-health-lock.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `fix(vpn): add concurrency lock to health check to prevent race (#26)`
  - Files: `src/vpn/health.rs`

- [x] 15. **#2 — MCP .mcp.json persistence: write config after add/remove**
  **What to do**:
  - In `crates/zeroclaw-mcp/src/config.rs`, add a `save_mcp_config(path: &Path, servers: &HashMap<String, McpServerConfig>)` function
  - Use atomic write pattern: write to `{path}.tmp`, then `std::fs::rename()` to final path
  - Serialize using existing `McpServerConfig` (already derives `Serialize`) into `{"mcpServers": {...}}` format
  - In `crates/zeroclaw-mcp/src/registry.rs`, after successful `add_server` and `remove_server`, call `save_mcp_config`
  - In `src/tools/mcp_manage.rs`, the `config_path` field is `#[allow(dead_code)]` — remove that attribute and pass it to registry for persistence
  **Must NOT do**:
  - Do not add auto-load on startup (out of scope)
  - Do not change the config read path
  - Do not add file watching for config changes
  - Do not add new crate dependencies (use `std::fs` for atomic write, or `tempfile` only if already in deps)
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 16-17)
  - **Blocks**: None
  - **Blocked By**: Tasks 9, 10, 11, 12 (all touch registry.rs)
  **References**:
  - `crates/zeroclaw-mcp/src/config.rs` (224 lines) — existing `parse_mcp_config()`, `load_mcp_configs()`, `McpConfigFile` struct
  - `crates/zeroclaw-mcp/src/registry.rs` — `add_server`, `remove_server` methods
  - `src/tools/mcp_manage.rs:154` — `config_path` field with `#[allow(dead_code)]`
  - `McpServerConfig` already derives `Serialize` — reuse for JSON output
  - MCP spec: atomic writes recommended (temp file + rename)
  **Acceptance Criteria**:
  - [ ] `save_mcp_config` function exists in config.rs
  - [ ] Atomic write pattern (temp + rename)
  - [ ] Called after add_server and remove_server
  - [ ] `#[allow(dead_code)]` removed from config_path
  - [ ] Output format matches `{"mcpServers": {...}}`
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Config persistence function exists
    Tool: Bash (grep + cargo)
    Steps:
      1. grep -n 'save_mcp_config\|persist' crates/zeroclaw-mcp/src/config.rs — verify function exists
      2. grep -n 'rename\|tmp' crates/zeroclaw-mcp/src/config.rs — verify atomic write
      3. grep -n 'save_mcp_config\|persist' crates/zeroclaw-mcp/src/registry.rs — verify called after add/remove
      4. grep -n 'allow(dead_code)' src/tools/mcp_manage.rs — should return NO matches
      5. cargo build 2>&1 — confirm compilation
    Expected Result: Persistence function with atomic write, called after add/remove
    Evidence: .sisyphus/evidence/task-15-mcp-persist.txt
  ```
  **Commit**: YES (groups with Wave 3)
  - Message: `fix(mcp): persist .mcp.json after add/remove with atomic writes (#2)`
  - Files: `crates/zeroclaw-mcp/src/config.rs`, `crates/zeroclaw-mcp/src/registry.rs`, `src/tools/mcp_manage.rs`
- [x] 16. **#32 — Feishu token cache TOCTOU: double-checked locking**
  **What to do**:
  - In `src/docs_sync/client.rs`, find `get_token()` method (around lines 59-113)
  - Current pattern: read lock → check expiry → drop read → write lock → refresh. This has a TOCTOU race where multiple concurrent callers all see expired token and all refresh.
  - Fix with double-checked locking pattern:
    1. Read lock: check if token is valid → return if yes
    2. Drop read lock
    3. Write lock: re-check if token is still expired (another task may have refreshed)
    4. Only refresh if still expired
  - The `CachedToken` struct uses `Instant` for `refresh_after` (already correct, no SystemTime issue here)
  **Must NOT do**:
  - Do not add external caching crate (moka, cached, etc.)
  - Do not change the token refresh HTTP call itself
  - Do not change the RwLock to Mutex (RwLock is correct for read-heavy pattern)
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15, 17)
  - **Blocks**: Task 17 (shares client.rs)
  - **Blocked By**: None
  **References**:
  - `src/docs_sync/client.rs:59-113` — `get_token()` method with the TOCTOU race
  - `src/docs_sync/client.rs:64` — `CachedToken` struct with `Instant`-based `refresh_after`
  - `src/channels/lark.rs:861-908` — similar token caching pattern in Lark channel (context, but do NOT modify)
  - Librarian research: double-checked locking with `tokio::sync::RwLock` is the idiomatic Rust pattern
  **Acceptance Criteria**:
  - [ ] Write lock path re-checks token validity before refreshing
  - [ ] No redundant refresh when multiple callers race
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Double-checked locking in get_token
    Tool: Bash (grep + read)
    Steps:
      1. Read src/docs_sync/client.rs get_token method
      2. Verify write lock path contains a re-check of token validity
      3. Look for pattern: write lock → check if still expired → only then refresh
      4. cargo build 2>&1 — confirm compilation
    Expected Result: Write lock path re-checks expiry before refreshing
    Evidence: .sisyphus/evidence/task-16-token-toctou.txt
  ```
  **Commit**: YES (groups with Wave 3)
  - Message: `fix(docs_sync): fix token cache TOCTOU with double-checked locking (#32)`
  - Files: `src/docs_sync/client.rs`
- [x] 17. **#31 — Feishu Docs Client: retry with exponential backoff**
  **What to do**:
  - In `src/docs_sync/client.rs`, add retry logic to HTTP methods: `get_raw_content`, `batch_update_blocks`, `create_document`
  - Follow the EXISTING retry pattern from `src/providers/reliable.rs`:
    - Exponential backoff: base 1s, multiplier 2x, cap 10s
    - Max 3 retries
    - Classify: 429 and 5xx as retryable, 4xx (except 429) as non-retryable
    - Parse `Retry-After` header if present on 429 responses
  - Implement as a helper function `retry_request` in client.rs (do NOT extract to shared util)
  - Add `tokio::time::sleep` between retries
  **Must NOT do**:
  - Do NOT add `reqwest-retry`, `reqwest-middleware`, or any external retry crate
  - Do not change the request construction or response parsing
  - Do not retry on auth errors (401/403)
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: `[]`
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 15-16)
  - **Blocks**: None
  - **Blocked By**: Task 16 (shares client.rs)
  **References**:
  - `src/docs_sync/client.rs` (211 lines) — `get_raw_content`, `batch_update_blocks`, `create_document` methods
  - `src/providers/reliable.rs` — EXISTING retry pattern to follow (exponential backoff, error classification, Retry-After parsing)
  - Do NOT look at external crates — follow the in-repo pattern only
  **Acceptance Criteria**:
  - [ ] Retry helper function exists in client.rs
  - [ ] Exponential backoff: 1s base, 2x multiplier, 10s cap
  - [ ] Max 3 retries
  - [ ] 429/5xx retried, 4xx (except 429) not retried
  - [ ] No new external crate dependencies
  - [ ] `cargo build` succeeds
  **QA Scenarios:**
  ```
  Scenario: Retry logic present in Feishu client
    Tool: Bash (grep)
    Steps:
      1. grep -n 'retry\|backoff\|Retry-After' src/docs_sync/client.rs — verify retry logic
      2. grep -n '429\|5[0-9][0-9]\|retryable' src/docs_sync/client.rs — verify error classification
      3. grep -rn 'reqwest-retry\|reqwest-middleware' Cargo.toml — should return NO matches (no new crate)
      4. cargo build 2>&1 — confirm compilation
    Expected Result: Retry helper with backoff, error classification, no new crates
    Evidence: .sisyphus/evidence/task-17-feishu-retry.txt
  ```
  **Commit**: YES (groups with Wave 3)
  - Message: `fix(docs_sync): add retry with exponential backoff for Feishu API calls (#31)`
  - Files: `src/docs_sync/client.rs`
---

## Final Verification Wave

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review all changed files for: `unwrap()` in non-test code, empty catches, `#[allow(unused)]` additions, commented-out code. Check AI slop: excessive comments, over-abstraction, generic variable names.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [x] F3. **Behavioral QA** — `unspecified-high`
  Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test cross-task integration where applicable (e.g., MCP fixes #1/#4/#5/#7 working together). Save to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [x] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git diff). Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Detect cross-task contamination. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

Each wave gets one commit after all tasks in the wave pass QA:

- **Wave 1**: `fix(skills,hooks,mcp): wave-1 isolated defect fixes (#10-13,#17-18,#20,#33,#5)` — skills/mod.rs, hooks/manifest.rs, hooks/dynamic.rs, hooks/reload.rs, docs_sync/watcher.rs, mcp/registry.rs
- **Wave 2**: `fix(mcp,vpn): wave-2 error handling and concurrency fixes (#1,#4,#7,#22,#26)` — mcp/registry.rs, mcp/client.rs, vpn/runtime.rs, vpn/health.rs
- **Wave 3**: `fix(mcp,feishu): wave-3 persistence, token cache, and retry fixes (#2,#31,#32)` — mcp/registry.rs, mcp/config.rs, mcp_manage.rs, docs_sync/client.rs

Pre-commit for all: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`

---

## Success Criteria

### Verification Commands
```bash
cargo fmt --all -- --check    # Expected: no output (clean)
cargo clippy --all-targets -- -D warnings  # Expected: no warnings
cargo test                     # Expected: all tests pass
```

### Final Checklist
- [ ] All 17 defects fixed
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] No new external crates added (except possibly `tempfile` if not already present)
- [ ] Error messages are actionable (include context for zeroclaw agent)
