## Initial Context

- Plan: skill-hotload-crud (7 tasks + 4 final verification)
- TDD approach: RED → GREEN → REFACTOR
- Key files: src/config/schema.rs, src/skills/mod.rs, src/tools/skill_manage.rs (new), src/tools/mod.rs, src/agent/loop_.rs
- User constraints: no security audit needed, device fully belongs to ZeroClaw when enabled
- Must use tokio::sync::RwLock (not std::sync) for shared state

## Task 3: SkillsState + reload_skills

- load_skills_with_config() takes &Path and &crate::config::Config (full Config, not SkillsConfig)
- Edit tool silently fails on src/skills/mod.rs (CRLF file on Windows) — use Python file I/O as workaround
- Pre-existing compilation breakage in providers/external.rs, providers/gemini.rs, gateway/, config/schema.rs blocks cargo test
- cargo check --lib confirms zero errors from skills module changes (all errors are in providers/ and gateway/)
- Linker PDB limit (LNK1318) on Windows debug builds blocks test binary linking for this large codebase
- SkillsState placed after SkillTool struct (line 46), reload_skills after load_skills_with_config (line 86)
- Tests follow existing pattern: tempfile::tempdir(), fs::create_dir_all, fs::write SKILL.toml, load_skills/reload_skills
- external.rs is untracked but referenced from providers/mod.rs — pre-existing parallel work breakage

## Task 1: skip_security_audit config field

- Field, env override, startup warning, and all 4 tests were ALREADY implemented in schema.rs
- Field at line 544: `pub skip_security_audit: bool` with `#[serde(default)]`
- Env override at lines 4248-4262: follows exact ZEROCLAW_OPEN_SKILLS_ENABLED pattern
- Startup warning at line 4254: emitted inside the `true` match arm (not separate if block)
- Tests at lines 7202-7239: default false, TOML parse true, absent defaults false, env override
- cargo check --bin zeroclaw passes clean
- cargo test --lib release build timed out (300s) due to full recompilation from scratch on Windows
- Linker PDB limit (LNK1318) still blocks debug test builds on this Windows machine

## 2026-02-26: skip_security_audit Flag Implementation

### Pattern: Conditional Security Audit Bypass

Successfully implemented `skip_security_audit` config flag to bypass skill security audits on trusted devices.

**Key Implementation Details:**

1. **Function Signature Changes**: Added `skip_audit: bool` parameter to:
   - `load_skills_from_directory()`
   - `load_open_skills()`
   - `load_workspace_skills()`
   - `load_skills_with_open_skills_config()`
   - `enforce_skill_security_audit()`
   - `install_local_skill_source()`
   - `install_git_skill_source()`

2. **Threading Pattern**: Config value flows from `config.skills.skip_security_audit` through:
   - `load_skills_with_config()` → `load_skills_with_open_skills_config()` → all downstream functions
   - `handle_command(Install)` → `install_*_skill_source()` → `enforce_skill_security_audit()`

3. **Conditional Logic**: When `skip_audit=true`:
   - `load_skills_from_directory()` and `load_open_skills()` skip the audit match entirely
   - `enforce_skill_security_audit()` returns `Ok(SkillAuditReport::default())` immediately
   - Skills load without any security validation

4. **CLI Audit Command Unaffected**: `handle_command(Audit)` directly calls `audit::audit_skill_directory()` without the skip parameter, ensuring user-initiated audits always run.

5. **Test Coverage**: Added 3 tests:
   - `load_skills_with_skip_audit_true_loads_dangerous_skill()` - verifies dangerous skills load when audit disabled
   - `load_open_skills_with_skip_audit_true_loads_dangerous_skill()` - same for open skills
   - `enforce_skill_security_audit_skip_audit_returns_clean_report()` - verifies early return behavior

**Lessons:**
- ast_grep_replace works well for single-function replacements but struggles with multi-statement patterns
- sed is effective for simple signature changes but leaves artifacts when patterns don't match exactly
- Manual Edit tool with precise LINE#ID references is most reliable for complex multi-line changes
- Always verify no duplicate code remnants after batch edits

**Files Modified:**
- `src/skills/mod.rs` - all audit call sites guarded with skip_audit parameter

## Task 3 Completion: SkillsState + reload_skills

- SkillsState struct placed after SkillTool (line 48-69), derives Debug+Clone, has `skills: Vec<Skill>` and `dirty: bool`
- Default impl delegates to `new()` which returns empty skills + dirty=false
- `reload_skills()` placed after `load_skills_with_config()` (line 112-116), takes `&mut SkillsState`, `&Path`, `&Config`
- 3 tests added: default state, reload populates from SKILL.toml on disk, reload resets dirty flag
- Edit tool worked correctly on this file this time (no CRLF issues encountered)
- `cargo check --bin zeroclaw` passes with zero errors
- File now has 1677 lines total

## Task 4: SkillManageTool (src/tools/skill_manage.rs)

- File created at 661 lines total, not yet registered in mod.rs (Task 5)
- Struct: `SkillManageTool` with `skills_dir`, `shared_state: Arc<RwLock<SkillsState>>`, `workspace_dir`, `config: Arc<Config>`
- Tool trait impl: name="skill_manage", 5 actions dispatched via match in execute()
- Name validation: manual char checks (no regex crate), rejects empty/long/non-alphanumeric-start/path-traversal/Windows-reserved
- TOML generation: custom `toml_quote()` helper for escaping strings, `build_toml()` for structured SKILL.toml output
- Delete action includes canonicalize + starts_with check for symlink escape prevention
- `reload_shared_state()` calls `crate::skills::reload_skills()` then sets `dirty = true`
- 11 tests total: 4 sync validation tests + 7 async action tests (create, duplicate, read, update, delete, list, roundtrip)
- `extract_name()` returns `Result<String, ToolResult>` — validation errors become Ok(ToolResult{success:false}) not Err
- `cargo check --bin zeroclaw` passes — file is unregistered so not compiled yet, but no regression
- LSP diagnostics unavailable on this Windows machine (rust-analyzer not detected by tool despite being installed)
