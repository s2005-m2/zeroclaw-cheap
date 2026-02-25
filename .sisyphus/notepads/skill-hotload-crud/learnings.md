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
