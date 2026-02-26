# Learnings — defect-fixes-17 Wave 1

## Fix C: AtomicBool migration
- `SkillsState.dirty` field used in 3 files: `skills/mod.rs`, `agent/loop_.rs`, `tools/skill_manage.rs`
- `SkillsState` derived `Clone` — had to implement `Clone` manually since `AtomicBool` doesn't derive it
- Used `Ordering::Relaxed` since dirty flag is advisory
- Two init sites in `loop_.rs`: one via `SkillsState::new()` (line ~2706) and one via struct literal (line ~2708)

## Fix A: tracing::warn on parse errors
- Replaced `if let Ok(skill)` with `match` to capture `Err(e)` — cleaner than double-calling the function
- Also added warning for `load_open_skill_md` in the open-skills loader (wasn't in original task but same pattern)

## Fix D: Blank lines
- Both blank line blocks (lines ~179-195 and ~272-288) were formatting artifacts, no deleted code per git log
- git log showed these were introduced in the same commit that added the security audit blocks

## Environment
- No rust-analyzer on this Windows machine — `cargo build --lib` used for verification
- Windows linker PDB error (LNK1318 LIMIT) is environment-specific, not code-related
- `cargo build --lib` passes clean with only pre-existing warnings
