# Learnings — brave-to-chromium task 1

## Edit tool range replace gotcha
- When using Edit tool with `pos`+`end` (was `pos`+`pos2`) for multi-line range replace, the old lines may not be fully removed — resulted in duplicate lines (old + new). Had to do a follow-up delete of leftover lines.
- Lesson: After range edits, always re-read and grep to verify no stale content remains.

## Windows MSVC LNK1318 PDB error
- This machine has a persistent `LNK1318: PDB error LIMIT (12)` linker issue that blocks `cargo test` and `cargo build` (debug profile).
- `cargo check --lib` works fine (skips linking).
- `cargo clean` does not fix it — it's a toolchain/environment issue.
- Workaround: use `cargo check` for compilation verification on this machine.

## Brave reference counts (baseline)
- `browser_open.rs`: was 11 Brave Browser refs, now 0 ✅
- `web_search_tool.rs`: 17 Brave Search API refs (untouched, correct)
- `schema.rs`: 20 case-insensitive brave refs remaining (all Brave Search config, 1 browser ref changed)
- The task predicted 14 for web_search and 11 for schema — actual counts differ slightly due to case-insensitive matching picking up more refs.

## Key distinction
- Two "Brave" concepts in codebase: Brave Browser (browser_open.rs) vs Brave Search API (web_search_tool.rs + schema.rs config). Only browser refs change.
