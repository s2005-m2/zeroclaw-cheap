# Replace Brave Browser with Chromium in browser_open

## TL;DR

> **Quick Summary**: Replace all Brave Browser references in `browser_open.rs` and 2 external files with Chromium equivalents. Brave Search API references remain untouched.
> 
> **Deliverables**:
> - `src/tools/browser_open.rs` — all 11 Brave Browser refs replaced with Chromium
> - `src/agent/loop_.rs` — tool description updated
> - `src/config/schema.rs` — config comment updated (line 1127 only)
> 
> **Estimated Effort**: Quick
> **Parallel Execution**: NO — 2 sequential waves (change → validate+commit)
> **Critical Path**: Task 1 → Task 2

---

## Context

### Original Request
User wants to completely replace Brave Browser references with Chromium. Brave Search API references (web_search_tool.rs, schema.rs search config) must NOT be changed — user only uses Bing for search.

### Interview Summary
**Key Discussions**:
- Two distinct "Brave" concepts identified: Brave Browser (desktop) vs Brave Search API (web search)
- User confirmed: Part 1 (browser) = full replacement; Part 2 (search API) = no change
- User only uses Bing for web search, so Brave Search API code is irrelevant but should remain functional

**Research Findings**:
- `browser_open.rs` has 11 Brave Browser string references across lines 7, 63, 72, 112, 115, 121, 127, 130, 145, 152, 169, 177
- `loop_.rs` line 2852 has 1 tool description reference
- `schema.rs` line 1127 has 1 config comment reference
- 17 tests in browser_open.rs — NONE assert on Brave-specific strings, all pass unchanged
- No Cargo.toml, CI workflow, or docs changes needed
- `browser.rs` already uses `native_chrome_path` — already Chromium-aligned
- No docs/ files reference "Brave Browser"

### Metis Review
**Identified Gaps** (addressed):
- Function rename decision: `open_in_brave` → `open_in_chromium` (Chromium-specific, matching the replacement intent)
- Platform binary names validated: macOS `["Chromium", "Google Chrome"]`, Linux `["chromium", "chromium-browser"]`, Windows `start "" chromium`
- Windows empty-string title argument in `start` command must be preserved
- Brave Search reference counts must be verified unchanged post-edit

---

## Work Objectives

### Core Objective
Replace all Brave Browser references with Chromium in the `browser_open` tool while preserving all existing security, validation, and error handling behavior.

### Concrete Deliverables
- `src/tools/browser_open.rs` — 11 string replacements + 1 function rename
- `src/agent/loop_.rs` — 1 tool description string replacement
- `src/config/schema.rs` — 1 doc comment replacement (line 1127 only)

### Definition of Done
- [ ] `cargo build --release` — zero errors
- [ ] `cargo test --lib -- browser_open` — 17 tests pass
- [ ] `cargo clippy --all-targets -- -D warnings` — zero warnings
- [ ] `cargo fmt --all -- --check` — clean
- [ ] Zero case-insensitive "brave" matches in `browser_open.rs`
- [ ] Brave Search reference counts unchanged in `web_search_tool.rs` (14) and `schema.rs` (11)

### Must Have
- All 11 Brave Browser string literals in `browser_open.rs` replaced
- Function `open_in_brave` renamed to `open_in_chromium`
- Tool description in `loop_.rs` updated
- Config comment in `schema.rs` updated
- Platform binary names: macOS `["Chromium", "Google Chrome"]`, Linux `["chromium", "chromium-browser"]`, Windows `chromium`

### Must NOT Have (Guardrails)
- MUST NOT touch `src/tools/web_search_tool.rs` — 14 Brave Search refs must remain
- MUST NOT touch `src/tools/mod.rs` line 295 — `brave_api_key` is Search API wiring
- MUST NOT touch `src/config/schema.rs` lines 1224-6823 — all Brave Search config refs
- MUST NOT change control flow, error handling structure, or `#[cfg(...)]` block structure
- MUST NOT change test assertions or test logic
- MUST NOT rename `BrowserOpenTool` struct (it's not Brave-specific)
- MUST NOT add new dependencies, config keys, or feature flags
- MUST NOT refactor to browser-agnostic config-driven approach (out of scope)
- MUST NOT touch `src/tools/browser.rs` (already Chromium-aligned)
- MUST NOT touch README.md or docs/ (no Brave Browser refs exist there)

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: Tests-after (existing tests cover validation; no new tests needed)
- **Framework**: `cargo test`

### QA Policy
Every task includes agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Code changes**: Use Bash — `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check`
- **Grep verification**: Use Bash — `grep -in "brave" src/tools/browser_open.rs` to confirm zero matches

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Start Immediately — primary code change):
└── Task 1: Replace Brave Browser → Chromium in 3 files [quick]

Wave 2 (After Wave 1 — validation + commit):
└── Task 2: Full validation suite + commit [quick, git-master]

Critical Path: Task 1 → Task 2
Max Concurrent: 1
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 2 | 1 |
| 2 | 1 | — | 2 |

### Agent Dispatch Summary

- **Wave 1**: 1 task — T1 → `quick`
- **Wave 2**: 1 task — T2 → `quick` + `git-master`

---

## TODOs


- [ ] 1. Replace Brave Browser with Chromium in browser_open.rs, loop_.rs, schema.rs

  **What to do**:
  - Rename function `open_in_brave` → `open_in_chromium` (line 127)
  - Replace all 11 Brave Browser string literals in `src/tools/browser_open.rs`:
    - Line 7: doc comment `/// Open approved HTTPS URLs in Brave Browser` → `/// Open approved HTTPS URLs in Chromium`
    - Line 63: `description()` return — "Brave Browser" → "Chromium"
    - Line 72: parameter schema description — "Brave Browser" → "Chromium"
    - Line 112: call site `open_in_brave(&url)` → `open_in_chromium(&url)`
    - Line 115: success message `"Opened in Brave: {url}"` → `"Opened in Chromium: {url}"`
    - Line 121: error message `"Failed to open Brave Browser: {e}"` → `"Failed to open Chromium: {e}"`
    - Line 127: function signature `async fn open_in_brave` → `async fn open_in_chromium`
    - Line 130 (macOS): `["Brave Browser", "Brave"]` → `["Chromium", "Google Chrome"]`
    - Line 145 (macOS error): update to mention Chromium/Google Chrome app names
    - Line 152 (Linux): `["brave-browser", "brave"]` → `["chromium", "chromium-browser"]`
    - Line 169 (Windows): `.args(["/C", "start", "", "brave", url])` → `.args(["/C", "start", "", "chromium", url])`
    - Line 177 (Windows error): update error message to mention chromium
  - Replace `src/agent/loop_.rs` line 2852: "Brave Browser" → "Chromium"
  - Replace `src/config/schema.rs` line 1127 ONLY: "Brave" → "Chromium"

  **Must NOT do**:
  - DO NOT touch `src/tools/web_search_tool.rs` (14 Brave Search refs)
  - DO NOT touch `src/tools/mod.rs` line 295 (`brave_api_key`)
  - DO NOT touch `src/config/schema.rs` lines 1224-6823 (Brave Search config)
  - DO NOT change control flow, `#[cfg(...)]` blocks, or error handling structure
  - DO NOT change any test assertions or test logic
  - DO NOT rename `BrowserOpenTool` struct
  - DO NOT add new dependencies, config keys, or feature flags
  - Preserve the empty-string title argument in Windows `start` command

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Scoped string replacements + 1 function rename across 3 files, no architectural changes
  - **Skills**: []
    - No specialized skills needed — pure text replacement in Rust source
  - **Skills Evaluated but Omitted**:
    - `git-master`: No git operations in this task
    - `playwright`: No browser automation needed

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 (solo)
  - **Blocks**: Task 2
  - **Blocked By**: None (can start immediately)

  **References**:

  **Pattern References**:
  - `src/tools/browser_open.rs:127-185` — `open_in_brave` function with all platform branches (macOS/Linux/Windows/#[cfg] blocks)
  - `src/tools/browser_open.rs:7-77` — struct, constructor, description, parameter schema
  - `src/tools/browser_open.rs:112-124` — execute() call site and result handling

  **API/Type References**:
  - `src/tools/browser_open.rs:8-11` — `BrowserOpenTool` struct (DO NOT rename)
  - `src/tools/traits.rs` — `Tool` trait that `BrowserOpenTool` implements

  **External References**:
  - macOS: `open -a "Chromium"` and `open -a "Google Chrome"` are the standard launch commands
  - Linux: `chromium` (Arch/Fedora) and `chromium-browser` (Debian/Ubuntu) are the standard binary names
  - Windows: `cmd /C start "" chromium` launches Chromium via shell

  **WHY Each Reference Matters**:
  - `browser_open.rs:127-185`: This is the core function being renamed — contains all platform-specific binary names that must change
  - `loop_.rs:2852`: Tool description shown to the LLM agent — must accurately describe what browser is used
  - `schema.rs:1127`: Config documentation — must match actual behavior

  **Acceptance Criteria**:

  **QA Scenarios (MANDATORY):**

  ```
  Scenario: Build succeeds with zero errors
    Tool: Bash
    Preconditions: All 3 files edited
    Steps:
      1. Run `cargo build --release 2>&1`
      2. Check exit code is 0
      3. Grep output for "error" — expect zero matches
    Expected Result: Clean build, exit code 0
    Failure Indicators: Any compilation error or unresolved reference to `open_in_brave`
    Evidence: .sisyphus/evidence/task-1-build.txt

  Scenario: All 17 browser_open tests pass
    Tool: Bash
    Preconditions: Build succeeds
    Steps:
      1. Run `cargo test --lib -- browser_open 2>&1`
      2. Parse output for test count and failures
    Expected Result: 17 tests pass, 0 failures
    Failure Indicators: Any test failure or panic
    Evidence: .sisyphus/evidence/task-1-tests.txt

  Scenario: Zero Brave Browser references remain in browser_open.rs
    Tool: Bash
    Preconditions: Edits complete
    Steps:
      1. Run `grep -in "brave" src/tools/browser_open.rs`
      2. Assert zero matches
    Expected Result: No output (zero matches)
    Failure Indicators: Any line containing "brave" (case-insensitive)
    Evidence: .sisyphus/evidence/task-1-no-brave.txt

  Scenario: Brave Search API references untouched
    Tool: Bash
    Preconditions: Edits complete
    Steps:
      1. Run `grep -c "brave" src/tools/web_search_tool.rs` — expect 14
      2. Run `grep -c "brave" src/config/schema.rs` — expect 11
    Expected Result: web_search_tool.rs=14, schema.rs=11
    Failure Indicators: Count differs from expected
    Evidence: .sisyphus/evidence/task-1-search-intact.txt
  ```

  **Evidence to Capture:**
  - [ ] task-1-build.txt — cargo build output
  - [ ] task-1-tests.txt — cargo test output
  - [ ] task-1-no-brave.txt — grep verification
  - [ ] task-1-search-intact.txt — Brave Search ref counts

  **Commit**: NO (grouped with Task 2)

---

- [ ] 2. Full validation suite + commit
  **What to do**:
  - Run `cargo fmt --all -- --check` — assert clean
  - Run `cargo clippy --all-targets -- -D warnings` — assert zero warnings
  - Run `cargo test` — assert all tests pass (not just browser_open)
  - Verify Brave Search reference counts: `grep -c "brave" src/tools/web_search_tool.rs` = 14, `grep -c "brave" src/config/schema.rs` = 11
  - Verify zero Brave refs in browser_open.rs: `grep -in "brave" src/tools/browser_open.rs` = 0
  - Create commit: `refactor(tools): replace Brave Browser with Chromium in browser_open`
  - Files in commit: `src/tools/browser_open.rs`, `src/agent/loop_.rs`, `src/config/schema.rs`
  **Must NOT do**:
  - DO NOT commit any files outside the 3 listed above
  - DO NOT amend existing commits
  - DO NOT push to remote
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Validation commands + single commit, no complex logic
  - **Skills**: [`git-master`]
    - `git-master`: Commit creation with proper conventional commit format
  - **Skills Evaluated but Omitted**:
    - `playwright`: No browser automation
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (solo)
  - **Blocks**: None
  - **Blocked By**: Task 1
  **References**:
  - `.githooks/` — Pre-push hook runs fmt+clippy+test
  - `AGENTS.md` section 6.1 — Branch/commit/PR flow requirements
  - `AGENTS.md` section 8 — Validation matrix: `cargo fmt`, `cargo clippy`, `cargo test`
  **Acceptance Criteria**:
  **QA Scenarios (MANDATORY):**
  ```
  Scenario: Full validation suite passes
    Tool: Bash
    Preconditions: Task 1 complete
    Steps:
      1. Run `cargo fmt --all -- --check` — expect exit 0
      2. Run `cargo clippy --all-targets -- -D warnings` — expect exit 0
      3. Run `cargo test` — expect all pass
    Expected Result: All three commands exit 0
    Failure Indicators: Any non-zero exit code
    Evidence: .sisyphus/evidence/task-2-validation.txt
  Scenario: Commit created with correct scope
    Tool: Bash
    Preconditions: Validation passes
    Steps:
      1. Run `git add src/tools/browser_open.rs src/agent/loop_.rs src/config/schema.rs`
      2. Run `git commit -m "refactor(tools): replace Brave Browser with Chromium in browser_open"`
      3. Run `git log -1 --oneline` — verify commit message
      4. Run `git diff --stat HEAD~1` — verify exactly 3 files changed
    Expected Result: Commit exists with correct message, 3 files changed
    Failure Indicators: Wrong file count, commit rejected by hook
    Evidence: .sisyphus/evidence/task-2-commit.txt
  ```
  **Evidence to Capture:**
  - [ ] task-2-validation.txt — fmt+clippy+test output
  - [ ] task-2-commit.txt — git log and diff stat
  **Commit**: YES
  - Message: `refactor(tools): replace Brave Browser with Chromium in browser_open`
  - Files: `src/tools/browser_open.rs`, `src/agent/loop_.rs`, `src/config/schema.rs`
  - Pre-commit: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`

---

## Final Verification Wave

> After ALL implementation tasks, run final checks.

- [ ] F1. **Brave Search Integrity Check** — `quick`
  Run `grep -c "brave" src/tools/web_search_tool.rs` and assert count = 14. Run `grep -c "brave" src/config/schema.rs` and assert count = 11 (was 12, minus 1 browser ref). If counts differ, the change touched Search API code — REJECT.
  Output: `web_search_tool.rs [14/14] | schema.rs [11/11] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Full Build + Test + Lint** — `quick`
  Run `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`. All must pass with zero errors/warnings.
  Output: `fmt [PASS/FAIL] | clippy [PASS/FAIL] | test [N pass/N fail] | VERDICT`

---

## Commit Strategy

- **Task 2**: `refactor(tools): replace Brave Browser with Chromium in browser_open` — `src/tools/browser_open.rs`, `src/agent/loop_.rs`, `src/config/schema.rs`
  - Pre-commit: `cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test`

---

## Success Criteria

### Verification Commands
```bash
cargo build --release              # Expected: zero errors
cargo test --lib -- browser_open   # Expected: 17 tests pass, 0 failures
cargo clippy --all-targets -- -D warnings  # Expected: zero warnings
cargo fmt --all -- --check         # Expected: clean
grep -in "brave" src/tools/browser_open.rs  # Expected: zero matches
grep -c "brave" src/tools/web_search_tool.rs  # Expected: 14 (unchanged)
grep -c "brave" src/config/schema.rs  # Expected: 11 (was 12, minus 1 browser ref)
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] Brave Search API fully intact
