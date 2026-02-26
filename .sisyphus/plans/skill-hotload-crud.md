# Skill 热加载 + CRUD Tool + 安全审计可跳过

## TL;DR

> **Quick Summary**: 为 ZeroClaw 新增 skill 运行时 CRUD 管理能力和热加载机制，让 agent 能自主创建/修改/删除 skill 并立即生效；同时新增全局开关跳过所有安全审计，实现设备完全归属 ZeroClaw。
> 
> **Deliverables**:
> - `src/config/schema.rs` — `SkillsConfig` 新增 `skip_security_audit` 字段
> - `src/skills/mod.rs` — 审计跳过逻辑 + `SkillsState` 共享类型 + `reload_skills()` 函数
> - `src/tools/skill_manage.rs` — 新 Tool 实现 CRUD 操作
> - `src/tools/mod.rs` — 注册 `SkillManageTool`
> - `src/agent/loop_.rs` — 共享 skills 状态 + 热加载 + system prompt 重建
> 
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 3 waves
> **Critical Path**: Task 1 → Task 4 → Task 6 → Task 7

---

## Context

### Original Request
用户需要 ZeroClaw agent 能在运行时自主创建和管理 skill，无需重启即可使用新 skill。同时要求所有安全审计可通过配置跳过，实现设备完全归属 ZeroClaw。

### Interview Summary
**Key Discussions**:
- 热加载策略：全量重载，加载期间 agent 等待（防死锁）
- CRUD 范围：create / read / update / delete / list 全部需要
- 审计跳过：全局一个开关，开启后整个设备归属 ZeroClaw
- 测试策略：TDD — 先写测试再实现

**Research Findings**:
- 6 个审计调用点全部在 `src/skills/mod.rs`，需要逐一加守卫
- Skills 在 agent loop 启动时一次性加载，注入 system prompt，无热加载机制
- Tool trait 的 `execute()` 只接收 `serde_json::Value`，工具需通过 Arc 自持状态
- `all_tools_with_runtime()` 返回 `Vec<Box<dyn Tool>>`，需新增参数传入共享状态
- 并行 tool 执行使用 `join_all()`，必须用 `tokio::sync::RwLock`（非 std::sync）

### Metis Review
**Identified Gaps** (addressed):
- Tool 如何获取共享 skills 状态 → 构造时注入 `Arc<RwLock<SkillsState>>`
- Agent loop 如何感知 skill 变更 → dirty flag 模式
- 热加载时 system prompt 重建需要保留原始构建参数
- `run_single_message` 不需要热加载（每次消息已重新加载）
- CLI `zeroclaw skills audit` 命令不受 skip 开关影响（用户主动诊断）
- Windows 路径和保留名处理
- Skill 名称冲突处理（已存在则报错）

---

## Work Objectives

### Core Objective
让 ZeroClaw agent 具备运行时 skill 自管理能力（CRUD + 热加载），并提供全局安全审计跳过开关。

### Concrete Deliverables
- `[skills] skip_security_audit = true` 配置项
- `skill_manage` tool（create/read/update/delete/list）
- 交互模式下 skill 热加载 + system prompt 自动重建
- 启动时安全审计跳过警告日志

### Definition of Done
- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo clippy --all-targets -- -D warnings` 通过
- [ ] `cargo test` 全部通过
- [ ] 新增 skill 后无需重启即可在下一轮对话中使用

### Must Have
- `skip_security_audit` 配置项，默认 false
- skill_manage tool 的 CRUD 全部操作
- 热加载机制（全量重载 + dirty flag）
- 路径遍历防护和 skill 名称验证
- TDD：每个功能先有测试

### Must NOT Have (Guardrails)
- skill_manage tool 不得执行 skill 中定义的命令 — 仅文件 CRUD
- 不得在 `run_tool_call_loop` 内部修改 — 仅在其返回后检查 dirty flag
- 不得给 `run_single_message` 添加热加载（它已经每次重载）
- 不得添加新 crate 依赖
- CLI `zeroclaw skills audit` 命令不受 skip 开关影响
- 不得创建过度抽象 — 保持 KISS/YAGNI

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: TDD (RED → GREEN → REFACTOR)
- **Framework**: `cargo test` (Rust built-in)
- **Each task follows**: Write failing test → Implement minimal code → Refactor

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Config/Library**: Use Bash (`cargo test`) — Run tests, assert pass counts
- **Integration**: Use Bash (`cargo build`, `cargo clippy`) — Verify compilation and lint

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately — foundation, MAX PARALLEL):
├── Task 1: skip_security_audit config field + defaults [quick]
├── Task 2: Audit bypass guards at 5 call sites in skills/mod.rs [business-logic]
└── Task 3: SkillsState shared type + reload_skills() function [business-logic]

Wave 2 (After Wave 1 — core implementation, MAX PARALLEL):
├── Task 4: skill_manage tool — CRUD implementation [deep]
└── Task 5: Tool registration + tool_descs in mod.rs [quick]

Wave 3 (After Wave 2 — integration):
├── Task 6: Agent loop integration — shared state + hot-reload + prompt rebuild [deep]
└── Task 7: Integration tests + full validation [business-logic]

Wave FINAL (After ALL tasks — independent review, 4 parallel):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
└── Task F4: Scope fidelity check (deep)

Critical Path: Task 1 → Task 4 → Task 6 → Task 7 → F1-F4
Parallel Speedup: ~40% faster than sequential
Max Concurrent: 3 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 2, 4, 5 | 1 |
| 2 | 1 | 6 | 1 |
| 3 | — | 4, 6 | 1 |
| 4 | 1, 3 | 5, 6 | 2 |
| 5 | 1, 4 | 6 | 2 |
| 6 | 2, 3, 4, 5 | 7 | 3 |
| 7 | 6 | — | 3 |

### Agent Dispatch Summary

- **Wave 1**: **3** — T1 → `quick`, T2 → `unspecified-high`, T3 → `unspecified-high`
- **Wave 2**: **2** — T4 → `deep`, T5 → `quick`
- **Wave 3**: **2** — T6 → `deep`, T7 → `unspecified-high`
- **FINAL**: **4** — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

> Implementation + Test = ONE Task. Never separate.
> EVERY task MUST have: Recommended Agent Profile + Parallelization info + QA Scenarios.
> TDD: Write failing test FIRST, then implement until green, then refactor.

- [x] 1. Add `skip_security_audit` to SkillsConfig

  **What to do**:
  - RED: Write test in `src/config/schema.rs` tests module: parse a TOML config with `[skills] skip_security_audit = true`, assert field is `true`. Write another test with field absent, assert default `false`.
  - RED: Write test that `SkillsConfig::default().skip_security_audit == false`.
  - GREEN: Add `#[serde(default)] pub skip_security_audit: bool` to `SkillsConfig` struct at `src/config/schema.rs:528`.
  - GREEN: Add `fn default_skip_security_audit() -> bool { false }` and wire it.
  - GREEN: Add env var override `ZEROCLAW_SKIP_SECURITY_AUDIT` in `apply_env_overrides()` (follow pattern of `ZEROCLAW_OPEN_SKILLS_ENABLED` at L4210).
  - GREEN: Add `tracing::warn!("Security audit for skills is DISABLED — device is fully trusted by ZeroClaw")` at startup when enabled (in the config loading path or agent init).
  - REFACTOR: Ensure JSON schema generation includes the new field.

  **Must NOT do**:
  - Do not modify any audit logic yet — that's Task 2
  - Do not add any other config fields

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single-file config schema change with clear pattern to follow
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - None applicable

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3)
  - **Blocks**: Tasks 2, 4, 5
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/config/schema.rs:528-540` — Current `SkillsConfig` struct definition, follow field pattern
  - `src/config/schema.rs:4210-4227` — `ZEROCLAW_OPEN_SKILLS_ENABLED` env override pattern to copy
  - `src/config/schema.rs:5985-6029` — Existing env override tests to follow as template

  **API/Type References**:
  - `src/config/schema.rs:507-521` — `SkillsPromptInjectionMode` enum, shows how config enums work

  **Test References**:
  - `src/config/schema.rs:4599-4602` — Existing `SkillsConfig` default assertions

  **WHY Each Reference Matters**:
  - L528-540: Copy the exact field annotation pattern (`#[serde(default)]`) for consistency
  - L4210-4227: The env override pattern is non-obvious (match on trimmed lowercase), must follow exactly
  - L5985-6029: Test structure shows how to set env vars, parse config, and assert — copy this pattern

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: `skip_security_audit` parses from TOML as `true`
  - [ ] Test: absent field defaults to `false`
  - [ ] Test: `SkillsConfig::default().skip_security_audit == false`
  - [ ] Test: env var `ZEROCLAW_SKIP_SECURITY_AUDIT=true` overrides config
  - [ ] `cargo test --lib -- config::tests` → PASS

  **QA Scenarios:**

  ```
  Scenario: Config field parses correctly
    Tool: Bash (cargo test)
    Preconditions: Tests written in src/config/schema.rs
    Steps:
      1. Run `cargo test --lib -- config::tests::skip_security_audit --nocapture`
      2. Assert exit code 0
      3. Assert output contains "test result: ok"
    Expected Result: All new tests pass
    Failure Indicators: Any test shows "FAILED" or exit code non-zero
    Evidence: .sisyphus/evidence/task-1-config-parse.txt

  Scenario: Compilation check
    Tool: Bash (cargo clippy)
    Preconditions: Field added to SkillsConfig
    Steps:
      1. Run `cargo clippy --all-targets -- -D warnings`
      2. Assert exit code 0
    Expected Result: No warnings or errors
    Failure Indicators: Any clippy warning about unused field or missing default
    Evidence: .sisyphus/evidence/task-1-clippy.txt
  ```

  **Commit**: YES
  - Message: `feat(config): add skip_security_audit toggle to SkillsConfig`
  - Files: `src/config/schema.rs`
  - Pre-commit: `cargo test --lib -- config::tests`

---

- [x] 2. Guard 5 audit call sites with `skip_security_audit` flag

  **What to do**:
  - RED: Write test `test_load_skills_skip_audit_allows_dangerous_skill` — create a skill dir with a `.sh` file (normally blocked), load with `skip_security_audit=true`, assert skill IS loaded.
  - RED: Write test `test_load_skills_audit_enabled_blocks_dangerous_skill` — same setup but `skip_security_audit=false`, assert skill is NOT loaded.
  - RED: Write test `test_install_local_skip_audit` — install a local skill with dangerous content when flag is true, assert success.
  - RED: Write test `test_install_git_skip_audit` — similar for git install path.
  - GREEN: Modify `load_skills_from_directory()` (L127): accept `skip_audit: bool` param. When true, skip the `audit::audit_skill_directory` call and proceed directly to loading.
  - GREEN: Modify `load_open_skills()` (L201): same pattern, skip `audit::audit_open_skill_markdown` when flag is true.
  - GREEN: Modify `enforce_skill_security_audit()` (L712): accept `skip_audit: bool`, return clean report immediately when true.
  - GREEN: Thread `skip_security_audit` from config through `load_skills_with_config()` → `load_skills_with_open_skills_config()` → `load_skills_from_directory()` and `load_open_skills()`.
  - GREEN: `install_local_skill_source()` (L784, L799) and `install_git_skill_source()` (L821): pass flag to `enforce_skill_security_audit()`.
  - IMPORTANT: Do NOT modify CLI `handle_command(Audit)` at L885 — that's user-initiated diagnostic, always runs.
  - REFACTOR: Clean up, ensure no dead code paths.

  **Must NOT do**:
  - Do not modify the CLI `zeroclaw skills audit` command behavior
  - Do not change audit.rs itself — only the call sites in mod.rs
  - Do not remove any existing audit logic, only add conditional bypass

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Touches 5+ locations in a single file with threading of a new parameter through call chain
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 3, after Task 1)
  - **Parallel Group**: Wave 1
  - **Blocks**: Task 6
  - **Blocked By**: Task 1 (needs `skip_security_audit` field in config)

  **References**:

  **Pattern References**:
  - `src/skills/mod.rs:127-144` — `load_skills_from_directory()` audit check to guard
  - `src/skills/mod.rs:201-218` — `load_open_skills()` audit check to guard
  - `src/skills/mod.rs:712-719` — `enforce_skill_security_audit()` to add skip param
  - `src/skills/mod.rs:775-806` — `install_local_skill_source()` two audit calls
  - `src/skills/mod.rs:808-828` — `install_git_skill_source()` audit call
  - `src/skills/mod.rs:80-86` — `load_skills_with_config()` entry point where config is available

  **API/Type References**:
  - `src/config/schema.rs:528` — `SkillsConfig` struct with `skip_security_audit` field (from Task 1)

  **Test References**:
  - `src/skills/mod.rs:1012-1060` — Existing skill loading tests to follow as template
  - `src/skills/audit.rs:490-598` — Existing audit tests showing how to create temp skill dirs

  **WHY Each Reference Matters**:
  - L127-144: This is the primary load path — must understand the match/Ok/Err pattern to add skip logic
  - L80-86: This is where config is available — must thread `skip_security_audit` from here downward
  - L1012-1060: Shows how to create temp dirs with SKILL.toml for testing

  **Acceptance Criteria**:

  **TDD:**
  - [ ] Test: dangerous skill loads when `skip_security_audit=true`
  - [ ] Test: dangerous skill blocked when `skip_security_audit=false`
  - [ ] Test: install local succeeds with dangerous content when flag true
  - [ ] Test: CLI audit command still works regardless of flag
  - [ ] `cargo test --lib -- skills::tests` → PASS

  **QA Scenarios:**
  ```
  Scenario: Audit bypass allows dangerous skill to load
    Tool: Bash (cargo test)
    Preconditions: Test creates temp skill dir with install.sh file
    Steps:
      1. Run `cargo test --lib -- skills::tests::test_load_skills_skip_audit --nocapture`
      2. Assert exit code 0
      3. Assert output contains "test result: ok"
    Expected Result: Skill with .sh file loads successfully when audit skipped
    Failure Indicators: Test FAILED or skill not found in loaded list
    Evidence: .sisyphus/evidence/task-2-audit-bypass.txt

  Scenario: Audit still blocks when flag is false
    Tool: Bash (cargo test)
    Preconditions: Same test setup but skip_security_audit=false
    Steps:
      1. Run `cargo test --lib -- skills::tests::test_load_skills_audit_enabled --nocapture`
      2. Assert exit code 0
    Expected Result: Dangerous skill is NOT loaded
    Failure Indicators: Skill appears in loaded list when it shouldn't
    Evidence: .sisyphus/evidence/task-2-audit-enforced.txt
  ```

  **Commit**: YES
  - Message: `feat(skills): guard audit call sites with skip_security_audit flag`
  - Files: `src/skills/mod.rs`
  - Pre-commit: `cargo test --lib -- skills::tests`

---

- [x] 3. SkillsState shared type + `reload_skills()` function
  **What to do**:
  - RED: Write test `test_skills_state_default` — `SkillsState::new()` has empty skills and `dirty=false`.
  - RED: Write test `test_reload_skills_populates_state` — create temp workspace with a valid skill, call `reload_skills()`, assert skills vec is non-empty and dirty is reset to false.
  - RED: Write test `test_reload_skills_with_skip_audit` — reload with dangerous skill + `skip_audit=true`, assert it loads.
  - GREEN: Define `SkillsState` struct in `src/skills/mod.rs`:
    ```rust
    pub struct SkillsState {
        pub skills: Vec<Skill>,
        pub dirty: bool,
    }
    ```
  - GREEN: Implement `SkillsState::new() -> Self` with empty defaults.
  - GREEN: Implement `pub fn reload_skills(state: &mut SkillsState, workspace_dir: &Path, config: &SkillsConfig)` — calls `load_skills_with_config()`, replaces `state.skills`, sets `dirty = false`.
  - REFACTOR: Ensure `SkillsState` derives `Debug, Clone`.
  **Must NOT do**:
  - Do not add Arc/RwLock here — that's the consumer's responsibility (Task 6)
  - Do not modify the agent loop
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: New shared type with reload logic, needs careful design for thread safety
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2)
  - **Blocks**: Tasks 4, 6
  - **Blocked By**: None
  **References**:
  **Pattern References**:
  - `src/skills/mod.rs:18-33` — `Skill` struct definition, the type stored in SkillsState
  - `src/skills/mod.rs:75-86` — `load_skills()` and `load_skills_with_config()` functions to call from reload
  **Test References**:
  - `src/skills/mod.rs:1012-1060` — Existing skill loading tests with temp dirs
  **WHY Each Reference Matters**:
  - L18-33: SkillsState wraps `Vec<Skill>`, must understand Skill's derive traits (Debug, Clone, Serialize, Deserialize)
  - L75-86: reload_skills() delegates to these functions, must match their signatures
  **Acceptance Criteria**:
  **TDD:**
  - [ ] Test: `SkillsState::new()` returns empty state with dirty=false
  - [ ] Test: `reload_skills()` populates skills from disk
  - [ ] Test: `reload_skills()` resets dirty to false
  - [ ] `cargo test --lib -- skills::tests::test_skills_state` → PASS
  **QA Scenarios:**
  ```
  Scenario: SkillsState reload from disk
    Tool: Bash (cargo test)
    Preconditions: Tests create temp workspace with valid SKILL.toml
    Steps:
      1. Run `cargo test --lib -- skills::tests::test_reload_skills --nocapture`
      2. Assert exit code 0
    Expected Result: Skills loaded from disk, dirty flag reset
    Failure Indicators: Empty skills vec or dirty still true
    Evidence: .sisyphus/evidence/task-3-reload.txt
  ```
  **Commit**: YES
  - Message: `feat(skills): add SkillsState shared type and reload_skills function`
  - Files: `src/skills/mod.rs`
  - Pre-commit: `cargo test --lib -- skills::tests`
---
- [x] 4. skill_manage tool — CRUD implementation
  **What to do**:
  - RED: Write tests in `src/tools/skill_manage.rs` module:
    - `test_create_skill` — call execute with `action=create, name="test-skill", description="A test"`, assert SKILL.toml written to disk, assert valid TOML, assert dirty=true on shared state.
    - `test_create_skill_path_traversal` — name `../../etc/passwd`, assert error.
    - `test_create_skill_windows_reserved` — name `CON`, `PRN`, assert error.
    - `test_create_skill_duplicate` — create same name twice, assert error on second.
    - `test_read_skill` — create then read, assert correct fields returned.
    - `test_update_skill` — create then update description, assert file changed.
    - `test_delete_skill` — create then delete, assert directory removed, dirty=true.
    - `test_list_skills` — create two skills, list, assert both returned.
    - `test_create_skill_toml_roundtrip` — create via tool, load via `load_skill_toml()`, assert match.
  - GREEN: Create `src/tools/skill_manage.rs` with `SkillManageTool` struct:
    ```rust
    pub struct SkillManageTool {
        skills_dir: PathBuf,
        shared_state: Arc<tokio::sync::RwLock<SkillsState>>,
        workspace_dir: PathBuf,
        config: Arc<crate::config::Config>,
    }
    ```
  - GREEN: Implement `Tool` trait:
    - `name()` → `"skill_manage"`
    - `description()` → `"Create, read, update, delete, and list agent skills at runtime"`
    - `parameters_schema()` → JSON schema with: `action` (required, enum: create/read/update/delete/list), `name` (required for CRUD, not for list), `description`, `version`, `tools` (array), `prompts` (array), `content` (string, for SKILL.md format)
    - `execute()` → dispatch on action parameter
  - GREEN: Implement each action:
    - **create**: Validate name (regex `^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$`, reject Windows reserved names CON/PRN/AUX/NUL/COM1-9/LPT1-9, reject path traversal). Create `skills_dir/<name>/` directory. Write `SKILL.toml` with provided fields. If `content` param provided, write `SKILL.md` instead. Acquire write lock on shared_state, call `reload_skills()`, set `dirty=true`, release lock.
    - **read**: Read `SKILL.toml` (or `SKILL.md`) from `skills_dir/<name>/`, return contents as JSON.
    - **update**: Validate name exists. Overwrite `SKILL.toml`/`SKILL.md` with new content. Reload + set dirty.
    - **delete**: Validate name exists and is inside skills_dir (canonicalize check). Remove directory. Reload + set dirty.
    - **list**: Read shared_state skills vec, return names + descriptions as JSON array.
  - REFACTOR: Extract name validation into a helper function.
  **Must NOT do**:
  - Do not execute any skill commands — file CRUD only
  - Do not call any audit functions
  - Do not add new crate dependencies
  - Do not create overly abstract builders — keep it direct
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Complex tool with 5 actions, validation logic, shared state interaction, and extensive TDD
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2
  - **Blocks**: Tasks 5, 6
  - **Blocked By**: Tasks 1 (config type), 3 (SkillsState type)
  **References**:
  **Pattern References**:
  - `src/tools/shell.rs:20-30` — Tool struct pattern with Arc fields
  - `src/tools/shell.rs:60-80` — `parameters_schema()` JSON schema pattern
  - `src/tools/shell.rs:82-170` — `execute()` pattern with args parsing and ToolResult return
  **API/Type References**:
  - `src/tools/traits.rs:1-43` — `Tool` trait, `ToolResult`, `ToolSpec` definitions
  - `src/skills/mod.rs:18-46` — `Skill`, `SkillTool` structs that SKILL.toml maps to
  - `src/skills/mod.rs:48-68` — `SkillManifest`, `SkillMeta` for TOML structure
  **Test References**:
  - `src/tools/traits.rs:45-80` — `DummyTool` test pattern
  - `src/skills/mod.rs:1012-1060` — `load_skill_from_toml()` test showing TOML format
  **External References**:
  - `tokio::sync::RwLock` docs — async read/write lock semantics
  **WHY Each Reference Matters**:
  - `shell.rs:20-30`: Shows how tools hold Arc<SecurityPolicy> — same pattern for Arc<RwLock<SkillsState>>
  - `traits.rs:1-43`: Must implement exactly this interface, return ToolResult with success/output/error
  - `skills/mod.rs:48-68`: The TOML structure that `load_skill_toml()` expects — created files must match
  **Acceptance Criteria**:
  **TDD:**
  - [ ] Test: create writes valid SKILL.toml
  - [ ] Test: path traversal rejected
  - [ ] Test: Windows reserved names rejected
  - [ ] Test: duplicate name rejected
  - [ ] Test: read returns correct data
  - [ ] Test: update modifies file
  - [ ] Test: delete removes directory
  - [ ] Test: list returns all skills
  - [ ] Test: TOML roundtrip (create → load_skill_toml)
  - [ ] `cargo test --lib -- tools::skill_manage::tests` → PASS
  **QA Scenarios:**
  ```
  Scenario: Full CRUD lifecycle
    Tool: Bash (cargo test)
    Preconditions: Tests use tempdir for skills_dir
    Steps:
      1. Run `cargo test --lib -- tools::skill_manage::tests --nocapture`
      2. Assert exit code 0
      3. Assert output contains "test result: ok" with 9+ tests
    Expected Result: All CRUD operations work correctly
    Failure Indicators: Any test FAILED
    Evidence: .sisyphus/evidence/task-4-crud.txt
  Scenario: Path traversal prevention
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test --lib -- tools::skill_manage::tests::test_create_skill_path_traversal --nocapture`
      2. Assert output contains "test result: ok"
    Expected Result: Names with ../ are rejected
    Failure Indicators: Skill created outside skills_dir
    Evidence: .sisyphus/evidence/task-4-path-traversal.txt
  ```
  **Commit**: YES
  - Message: `feat(tools): implement skill_manage CRUD tool`
  - Files: `src/tools/skill_manage.rs`
  - Pre-commit: `cargo test --lib -- tools::skill_manage::tests`
---
- [x] 5. Register skill_manage tool + add tool_descs entry
  **What to do**:
  - RED: Write test that `all_tools_with_runtime()` returns a tool named `"skill_manage"` when shared state is provided.
  - GREEN: Add `mod skill_manage;` to `src/tools/mod.rs`.
  - GREEN: Add `use skill_manage::SkillManageTool;` import.
  - GREEN: In `all_tools_with_runtime()` function signature, add parameter: `shared_skills: Option<Arc<tokio::sync::RwLock<crate::skills::SkillsState>>>`.
  - GREEN: After existing tool registrations, add conditional registration:
    ```rust
    if let Some(ref shared) = shared_skills {
        tool_arcs.push(Arc::new(SkillManageTool::new(
            skills_dir, shared.clone(), workspace_dir.to_path_buf(), root_config.clone(),
        )));
    }
    ```
  - GREEN: Add `skill_manage` to `tool_descs` in both `run_agent_loop` (L2774) and `run_single_message` (L3216):
    ```rust
    tool_descs.push(("skill_manage", "Create, read, update, delete, and list agent skills at runtime. Use to extend your own capabilities."));
    ```
  - GREEN: Update ALL existing call sites of `all_tools_with_runtime()` to pass `None` for the new parameter (except the ones we'll update in Task 6).
  - REFACTOR: Ensure no unused import warnings.
  **Must NOT do**:
  - Do not modify the agent loop logic — that's Task 6
  - Do not change any existing tool registrations
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Mechanical wiring — add import, add parameter, add registration
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO (needs Task 4's module)
  - **Parallel Group**: Wave 2 (after Task 4)
  - **Blocks**: Task 6
  - **Blocked By**: Tasks 1, 4
  **References**:
  **Pattern References**:
  - `src/tools/mod.rs:191-235` — `all_tools_with_runtime()` function signature and tool registration pattern
  - `src/tools/mod.rs:237-298` — Conditional tool registration pattern (browser, http, composio)
  - `src/agent/loop_.rs:2774-2818` — `tool_descs` list in `run_agent_loop`
  - `src/agent/loop_.rs:3216-3260` — `tool_descs` list in `run_single_message`
  **WHY Each Reference Matters**:
  - L191-235: Must match exact function signature pattern and Arc wrapping
  - L237-298: Shows how to conditionally register tools (if browser_config.enabled, etc.)
  - L2774-2818: Must add tool_descs entry here for interactive mode
  **Acceptance Criteria**:
  **TDD:**
  - [ ] Test: tool named `skill_manage` exists in registry when shared state provided
  - [ ] `cargo build` → SUCCESS (no compile errors from new parameter)
  - [ ] `cargo clippy --all-targets -- -D warnings` → PASS
  **QA Scenarios:**
  ```
  Scenario: Compilation with new tool registration
    Tool: Bash
    Steps:
      1. Run `cargo build 2>&1`
      2. Assert exit code 0
    Expected Result: Clean compilation
    Failure Indicators: Unresolved import or type mismatch errors
    Evidence: .sisyphus/evidence/task-5-build.txt
  ```
  **Commit**: YES
  - Message: `feat(tools): register skill_manage tool in tool factory`
  - Files: `src/tools/mod.rs`, `src/agent/loop_.rs`
  - Pre-commit: `cargo build`
---
- [x] 6. Agent loop integration — shared state + hot-reload + prompt rebuild
  **What to do**:
  - RED: Write test `test_hot_reload_updates_system_prompt` — simulate: create SkillsState with dirty=true and new skill, call the prompt rebuild logic, assert history[0] contains new skill name.
  - RED: Write test `test_hot_reload_skipped_when_not_dirty` — dirty=false, assert history[0] unchanged.
  - GREEN: In `run_agent_loop()` (src/agent/loop_.rs), BEFORE building tools_registry:
    1. Create `let shared_skills = Arc::new(tokio::sync::RwLock::new(SkillsState { skills: skills.clone(), dirty: false }));`
    2. Pass `Some(shared_skills.clone())` to `all_tools_with_runtime()`
  - GREEN: Retain all parameters needed for `build_system_prompt_with_mode()` rebuild: `workspace_dir`, `model_name`, `tool_descs`, `identity`, `bootstrap_max_chars`, `native_tools`, `prompt_injection_mode`. These are already local variables — just ensure they live long enough.
  - GREEN: In the interactive loop, AFTER `run_tool_call_loop()` returns (after L3092 area), add hot-reload check:
    ```rust
    // Hot-reload skills if dirty
    {
        let needs_reload = shared_skills.read().await.dirty;
        if needs_reload {
            let mut state = shared_skills.write().await;
            crate::skills::reload_skills(&mut state, &config.workspace_dir, &config);
            // Rebuild system prompt with updated skills
            let new_prompt = crate::channels::build_system_prompt_with_mode(
                &config.workspace_dir, model_name, &tool_descs, &state.skills,
                Some(&config.identity), bootstrap_max_chars, native_tools,
                config.skills.prompt_injection_mode,
            );
            if !native_tools {
                // re-append tool instructions if needed
            }
            history[0] = ChatMessage::system(&new_prompt);
            state.dirty = false;
        }
    }
    ```
  - GREEN: For `run_single_message()`: pass `None` for shared_skills (it reloads per-message already).
  - IMPORTANT: Use `tokio::sync::RwLock` (NOT `std::sync::RwLock`) because tool execution uses `join_all()` for parallel tools.
  - IMPORTANT: Keep lock scopes minimal — read lock only to check dirty flag, write lock only during reload. Never hold lock across await points in the main loop.
  - REFACTOR: Extract the hot-reload block into a helper function `maybe_reload_skills()` to keep the loop clean.
  **Must NOT do**:
  - Do not modify `run_tool_call_loop()` internals
  - Do not add hot-reload to `run_single_message()`
  - Do not hold RwLock across the entire tool call loop iteration
  - Do not add new crate dependencies
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: HIGH-RISK change touching core agent loop, requires careful async/lock reasoning
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (sequential)
  - **Blocks**: Task 7
  - **Blocked By**: Tasks 2, 3, 4, 5
  **References**:
  **Pattern References**:
  - `src/agent/loop_.rs:2773-2897` — Skills loading + system prompt construction in run_agent_loop
  - `src/agent/loop_.rs:2940-2974` — history construction with system_prompt
  - `src/agent/loop_.rs:2976-3095` — Interactive loop structure (where to insert reload check)
  - `src/agent/loop_.rs:3215-3280` — run_single_message skills loading (pass None here)
  **API/Type References**:
  - `src/channels/mod.rs` — `build_system_prompt_with_mode()` function signature
  - `src/skills/mod.rs` — `SkillsState`, `reload_skills()` from Task 3
  - `src/agent/loop_.rs:2888-2897` — Original `build_system_prompt_with_mode` call with all params
  **WHY Each Reference Matters**:
  - L2773-2897: Must understand the full setup sequence to know where to inject shared state creation
  - L2976-3095: The interactive loop body — reload check goes AFTER run_tool_call_loop returns, BEFORE next iteration
  - L2888-2897: The exact parameters for prompt rebuild — must retain these for hot-reload
  **Acceptance Criteria**:
  **TDD:**
  - [ ] Test: dirty=true triggers prompt rebuild with new skill
  - [ ] Test: dirty=false skips rebuild
  - [ ] `cargo test --lib -- agent::tests` → PASS
  - [ ] `cargo clippy --all-targets -- -D warnings` → PASS
  **QA Scenarios:**
  ```
  Scenario: Hot-reload integration compiles and passes
    Tool: Bash
    Steps:
      1. Run `cargo build 2>&1`
      2. Assert exit code 0
      3. Run `cargo test --lib -- agent::tests --nocapture 2>&1`
      4. Assert exit code 0
    Expected Result: Clean build and all agent tests pass
    Failure Indicators: Deadlock (timeout), compile error, test failure
    Evidence: .sisyphus/evidence/task-6-hotreload.txt
  Scenario: No deadlock under concurrent access
    Tool: Bash (cargo test)
    Steps:
      1. Run `cargo test --lib -- agent::tests::test_hot_reload --nocapture`
      2. Assert completes within 30 seconds (no hang)
    Expected Result: Test completes without timeout
    Failure Indicators: Process hangs or times out
    Evidence: .sisyphus/evidence/task-6-no-deadlock.txt
  ```
  **Commit**: YES
  - Message: `feat(agent): integrate skill hot-reload into agent loop`
  - Files: `src/agent/loop_.rs`
  - Pre-commit: `cargo test --lib -- agent::tests`
---
- [x] 7. Integration tests + full validation
  **What to do**:
  - Write integration tests that exercise the full end-to-end flow:
    - `test_e2e_create_skill_and_reload` — Create a skill via `SkillManageTool::execute()`, verify SKILL.toml on disk, verify `load_skill_toml()` can parse it, verify dirty flag set.
    - `test_e2e_skip_audit_loads_dangerous_skill` — Set `skip_security_audit=true` in config, create a skill dir with `.sh` file manually, call `load_skills_with_config()`, assert skill IS loaded.
    - `test_e2e_audit_enabled_blocks_dangerous_skill` — Same but `skip_security_audit=false`, assert skill NOT loaded.
    - `test_e2e_crud_lifecycle` — Create → Read → Update → List → Delete → List, verify each step.
    - `test_e2e_path_traversal_blocked` — Attempt create with `../../evil`, assert error.
    - `test_e2e_windows_reserved_names_blocked` — Attempt create with `CON`, `PRN`, `AUX`, `NUL`, assert error.
    - `test_e2e_skill_name_collision` — Create same name twice, assert second fails.
    - `test_e2e_empty_skills_dir` — skills dir doesn't exist, create skill, assert dir created.
  - Run full validation suite:
    ```bash
    cargo fmt --all -- --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    ```
  - Fix any failures found during full suite run.
  **Must NOT do**:
  - Do not add new features — only tests and fixes for existing implementation
  - Do not refactor working code unless tests reveal a bug
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Cross-module integration testing requiring understanding of all prior tasks
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (after Task 6)
  - **Blocks**: None (final implementation task)
  - **Blocked By**: Task 6
  **References**:
  **Pattern References**:
  - `src/skills/mod.rs:1012-1060` — Existing skill loading tests with tempdir pattern
  - `src/skills/audit.rs:490-598` — Audit tests showing tempdir + SKILL.md creation
  - `src/tools/skill_manage.rs` — The tool under test (from Task 4)
  **API/Type References**:
  - `src/skills/mod.rs` — `load_skills_with_config()`, `SkillsState`, `reload_skills()`
  - `src/config/schema.rs` — `SkillsConfig` with `skip_security_audit`
  **WHY Each Reference Matters**:
  - L1012-1060: Shows the tempdir pattern for creating test skill directories
  - audit.rs tests: Shows how to create skills that trigger audit findings
  **Acceptance Criteria**:
  **TDD:**
  - [ ] 8+ integration tests written and passing
  - [ ] `cargo test` — ALL tests pass (existing + new)
  - [ ] `cargo fmt --all -- --check` — clean
  - [ ] `cargo clippy --all-targets -- -D warnings` — clean
  **QA Scenarios:**
  ```
  Scenario: Full test suite passes
    Tool: Bash
    Steps:
      1. Run `cargo fmt --all -- --check`
      2. Assert exit code 0
      3. Run `cargo clippy --all-targets -- -D warnings`
      4. Assert exit code 0
      5. Run `cargo test 2>&1`
      6. Assert exit code 0
      7. Assert output contains "test result: ok"
    Expected Result: Zero failures across fmt, clippy, and test
    Failure Indicators: Any non-zero exit code or "FAILED" in output
    Evidence: .sisyphus/evidence/task-7-full-suite.txt
  Scenario: Integration tests specifically pass
    Tool: Bash
    Steps:
      1. Run `cargo test --lib -- test_e2e --nocapture 2>&1`
      2. Assert exit code 0
      3. Assert 8+ tests in output
    Expected Result: All e2e tests pass
    Failure Indicators: Any test FAILED
    Evidence: .sisyphus/evidence/task-7-e2e.txt
  ```
  **Commit**: YES
  - Message: `test(skills): add integration tests for skill CRUD + hot-reload + audit bypass`
  - Files: `src/skills/mod.rs`, `src/tools/skill_manage.rs`
  - Pre-commit: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`
## Final Verification Wave

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review all changed files for: `as any`/`unwrap()` in non-test code, empty catches, `println!` in prod, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction, generic names.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Execute EVERY QA scenario from EVERY task — follow exact steps, capture evidence. Test cross-task integration (skill create → hot-reload → prompt contains new skill). Test edge cases: empty state, invalid input, path traversal. Save to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git log/diff). Verify 1:1 — everything in spec was built (no missing), nothing beyond spec was built (no creep). Check "Must NOT do" compliance. Detect cross-task contamination. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **T1**: `feat(config): add skip_security_audit toggle to SkillsConfig` — `src/config/schema.rs`
- **T2**: `feat(skills): guard audit call sites with skip_security_audit flag` — `src/skills/mod.rs`
- **T3**: `feat(skills): add SkillsState shared type and reload_skills function` — `src/skills/mod.rs`
- **T4**: `feat(tools): implement skill_manage CRUD tool` — `src/tools/skill_manage.rs`
- **T5**: `feat(tools): register skill_manage tool in tool factory` — `src/tools/mod.rs`
- **T6**: `feat(agent): integrate skill hot-reload into agent loop` — `src/agent/loop_.rs`
- **T7**: `test(skills): add integration tests for skill CRUD + hot-reload + audit bypass` — `src/skills/`, `src/tools/`

---

## Success Criteria

### Verification Commands
```bash
cargo fmt --all -- --check        # Expected: no output (clean)
cargo clippy --all-targets -- -D warnings  # Expected: no warnings
cargo test                        # Expected: all tests pass
cargo test -- skill_manage        # Expected: CRUD tests pass
cargo test -- skip_security_audit # Expected: audit bypass tests pass
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] `skip_security_audit = true` skips audit at 5 call sites
- [ ] `skill_manage create` writes valid SKILL.toml
- [ ] Hot-reload updates system prompt after skill change
- [ ] Path traversal attacks blocked
- [ ] Startup warning logged when audit skipped
