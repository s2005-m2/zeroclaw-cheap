# Feishu Reply + Streaming Card Tool Notifications (REVISED)

## TL;DR

> **Quick Summary**: Split Feishu message flow into two paths: (1) plain text reply via reply API for final responses, (2) independent streaming card for tool execution progress. Fix the broken streaming card element-level update bug.
> 
> **Deliverables**:
> - Plain text reply-linked responses for P2P messages via `POST /im/v1/messages/:message_id/reply`
> - Working streaming cards with element-level updates for tool execution progress only
> - Flow split in `channels/mod.rs` so final response always goes through `send()` (not `finalize_draft`)
> - Re-enabled `stream_mode` from config (currently force-disabled)
> 
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 3 waves
> **Critical Path**: Task 1 → Task 2 → Task 3 → Task 5 → Task 6 → F1-F3

---

## Context

### Original Request
User wants ZeroClaw to:
1. Reply to Feishu messages using the reply API — final response as plain text (NOT card), linked to user's original message
2. Show tool/shell execution notifications via independent streaming cards (NOT reply-linked)
3. Fix the broken streaming card bug

### Interview Summary
**Key Discussions**:
- Reply scope: P2P (1:1) messages only — group chats NOT in scope
- Final response: plain text message via reply API (`reply_in_thread: true`), NOT a card
- Tool notifications: independent streaming card, NOT reply-linked to user message
- `send_draft`/`update_draft`/`finalize_draft`: repurposed for tool notification cards only — no longer streams final response
- `channels/mod.rs` must be modified to split the flow (was previously off-limits)
- `agent/loop_.rs` stays completely unchanged

**Research Findings**:
- Streaming card bug: whole-card update (`PUT /cardkit/v1/cards/:card_id`) fails with "card is required"
- Fix: element-level update (`PUT /cardkit/v1/cards/:card_id/elements/content/content`)
- OpenClaw reference confirms element-level update pattern with `streaming_mode`, `element_id`, `streaming_config`
- `ChannelMessage.thread_ts` already exists, just needs population for Lark P2P
- `SendMessage.thread_ts` and `.in_thread()` already exist
- Tool notification flow in `agent/loop_.rs` sends `⏳ tool: hint` and `✅/❌ tool (Xs)` via `delta_tx`

### Metis Review
**Identified Gaps** (addressed):
- Draft updater continues updating card with final response text after CLEAR sentinel — need stop-flag
- `finalize_draft()` contract: Telegram depends on receiving full response text — Lark must ignore it
- `send()` currently sends cards (`msg_type: "interactive"`), not plain text — need reply path with `msg_type: "text"`
- Error/timeout paths also call `finalize_draft()` — must handle correctly
- "No tools called" edge case: streaming card created then immediately closed with no content
- Close streaming via `PATCH /cardkit/v1/cards/:card_id/settings` (not a card update)

---

## Work Objectives

### Core Objective
Split Feishu message delivery into two paths: plain text reply for final responses, independent streaming card for tool progress.

### Concrete Deliverables
- Modified `src/channels/lark.rs` with reply API, streaming card fix, thread_ts population
- Modified `src/channels/mod.rs` with flow split: finalize_draft closes card, send() delivers reply
- Unit tests for new reply and streaming paths

### Definition of Done
- [ ] `cargo test` passes with all new + existing tests
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `grep -n "StreamMode::Off" src/channels/lark.rs` returns 0 matches in `from_*config()` methods
- [ ] `grep -n "reply_in_thread" src/channels/lark.rs` returns ≥1 match
- [ ] `grep -n "element_id" src/channels/lark.rs` returns ≥1 match
- [ ] `grep -n "streaming_mode" src/channels/lark.rs` returns ≥1 match
- [ ] `grep -n "msg_type.*text" src/channels/lark.rs` returns ≥1 match (plain text reply)

### Must Have
- Reply API for P2P messages with `reply_in_thread: true` and `msg_type: "text"`
- Element-level CardKit update (not whole-card update)
- `streaming_mode: true` in card config
- `close_streaming()` called in `finalize_draft()`
- `stream_mode` read from config instead of forced `Off`
- Draft updater stop-flag after CLEAR sentinel (no final response in card)
- `send()` always called for final response delivery (with `thread_ts` for reply)

### Must NOT Have (Guardrails)
- DO NOT modify `agent/loop_.rs` — zero changes to delta_tx protocol, CLEAR sentinel, or tool progress format
- DO NOT modify `Channel` trait or `ChannelMessage` struct
- DO NOT add reply support for group chats — P2P only
- DO NOT add new config schema keys
- DO NOT change `listen_http()` webhook handler
- DO NOT change typing indicator cards (they work fine with whole-card update)
- DO NOT break Telegram's `finalize_draft()` behavior (it depends on receiving full response text)
- DO NOT break Slack/Discord default no-op draft paths
- DO NOT add channel-specific `if channel.name() == "lark"` checks in mod.rs — use capability/state checks
- DO NOT add `reply_in_thread` as a config option — always `true`, hardcoded
- DO NOT add markdown-to-rich-text conversion for plain text reply

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (`cargo test`)
- **Automated tests**: YES (tests-after)
- **Framework**: `cargo test` (Rust built-in)

### QA Policy
Every task includes agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Library/Module**: Use Bash (`cargo test`, `cargo clippy`, `grep`) to verify

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — streaming card fix in lark.rs):
├── Task 1: Fix create_card() with streaming config + element_id [quick]
├── Task 2: Replace update_card() with element-level update + add close_streaming() [quick]

Wave 2 (Reply API + thread_ts in lark.rs):
├── Task 3: Thread message_id into ChannelMessage.thread_ts for P2P [quick]
├── Task 4: Add reply_text() method and reply path in send() [quick]

Wave 3 (Flow split + re-enable — mod.rs + lark.rs):
├── Task 5: Modify channels/mod.rs: stop-flag in draft_updater + always send() for final response [deep]
├── Task 6: Re-enable stream_mode from config + finalize_draft closes card + unit tests [quick]

Wave FINAL (Verification):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Scope fidelity check (deep)

Critical Path: Task 1 → Task 2 → Task 3 → Task 5 → Task 6 → F1-F3
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | — | 2, 6 |
| 2 | 1 | 6 |
| 3 | — | 4, 5 |
| 4 | 3 | 5 |
| 5 | 4 | 6 |
| 6 | 1, 2, 5 | F1-F3 |
| F1-F3 | 6 | — |

### Agent Dispatch Summary

- **Wave 1**: 2 tasks — T1 → `quick`, T2 → `quick`
- **Wave 2**: 2 tasks — T3 → `quick`, T4 → `quick`
- **Wave 3**: 2 tasks — T5 → `deep`, T6 → `quick`
- **FINAL**: 3 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `deep`

---

## TODOs

- [ ] 1. Fix `create_card()` with streaming config and element_id

  **What to do**:
  - Add constant `STREAMING_ELEMENT_ID: &str = "content"` near the top of `lark.rs`
  - In `send_draft()` (line 1436), update the card JSON to include:
    - `"config": { "streaming_mode": true, "streaming_config": { "print_frequency_ms": 100, "print_count_per_time": 5 }, "summary": { "content": "..." } }`
    - `"element_id": "content"` on the markdown element
  - Do NOT change `create_card()` method signature — it just forwards the JSON
  - Do NOT change `start_typing()` card JSON — typing cards are not streaming cards

  **Must NOT do**:
  - Do not change `create_card()` method signature or logic
  - Do not modify `start_typing()` or `stop_typing()`
  - Do not modify any other methods in this task

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 (sequential with Task 2)
  - **Blocks**: Task 2, Task 6
  - **Blocked By**: None

  **References**:
  - `src/channels/lark.rs:1425-1472` — `send_draft()` method with card JSON at lines 1436-1445
  - `src/channels/lark.rs:1176-1201` — `create_card()` method (unchanged, just forwards JSON)
  - `src/channels/lark.rs:1282-1313` — `start_typing()` card JSON pattern (DO NOT modify)
  - OpenClaw reference: card JSON must have `streaming_mode: true`, `element_id` on elements, `streaming_config`

  **Acceptance Criteria**:
  - [ ] `grep -n "streaming_mode" src/channels/lark.rs` returns ≥1 match in `send_draft`
  - [ ] `grep -n "element_id" src/channels/lark.rs` returns ≥1 match in `send_draft`
  - [ ] `grep -n "streaming_config" src/channels/lark.rs` returns ≥1 match
  - [ ] `cargo test -- lark` passes

  **QA Scenarios**:
  ```
  Scenario: send_draft card JSON includes streaming config
    Tool: Bash (grep)
    Steps:
      1. grep -n "streaming_mode" src/channels/lark.rs
      2. grep -n "element_id" src/channels/lark.rs
      3. grep -n "streaming_config" src/channels/lark.rs
      4. Verify matches are inside send_draft() method (around line 1436)
    Expected Result: All three patterns found in send_draft card JSON
    Evidence: .sisyphus/evidence/task-1-streaming-config.txt

  Scenario: start_typing card JSON unchanged
    Tool: Bash (grep)
    Steps:
      1. grep -A5 "start_typing" src/channels/lark.rs | grep -c "streaming_mode"
    Expected Result: 0 matches — start_typing does not have streaming_mode
    Evidence: .sisyphus/evidence/task-1-typing-unchanged.txt
  ```

  **Commit**: NO (groups with Task 2)

- [ ] 2. Replace `update_card()` with element-level update + add `close_streaming()`

  **What to do**:
  - Rename existing `update_card()` to `update_card_whole()` (used only by `stop_typing()`)
  - Create new `update_card_element()` method:
    - `PUT /cardkit/v1/cards/{card_id}/elements/{STREAMING_ELEMENT_ID}/content`
    - Body: `{ "content": markdown_text, "sequence": N, "uuid": "s_{card_id}_{N}" }`
    - Follow existing token-refresh-retry pattern from `send_card_message()` (lines 1243-1276)
  - Create new `close_streaming()` method:
    - `PATCH /cardkit/v1/cards/{card_id}/settings`
    - Body: `{ "settings": JSON.stringify({"config":{"streaming_mode":false,"summary":{"content":summary}}}), "sequence": N, "uuid": "c_{card_id}_{N}" }`
    - Uses `self.http_client().patch(&url)` with token auth
  - Update `update_draft()` (line 1473) to call `update_card_element()` instead of `update_card()`
    - Pass just the markdown text, not a full card JSON
  - Update `stop_typing()` (line 1317) to call `update_card_whole()` (renamed original)

  **Must NOT do**:
  - Do not change `start_typing()` — it creates cards, doesn't update them
  - Do not change `create_card()` — it creates cards
  - Do not change `send_card_message()` — it sends card references
  - Do not change `finalize_draft()` yet — that's Task 6

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 (after Task 1)
  - **Blocks**: Task 6
  - **Blocked By**: Task 1

  **References**:
  - `src/channels/lark.rs:1203-1241` — current `update_card()` method (to be renamed)
  - `src/channels/lark.rs:1317-1341` — `stop_typing()` calls `update_card()` (must call renamed method)
  - `src/channels/lark.rs:1473-1514` — `update_draft()` calls `update_card()` (must call new element method)
  - `src/channels/lark.rs:1243-1276` — `send_card_message()` token-refresh-retry pattern to follow
  - OpenClaw element update: `PUT /cardkit/v1/cards/:card_id/elements/content/content`
  - OpenClaw close streaming: `PATCH /cardkit/v1/cards/:card_id/settings`

  **Acceptance Criteria**:
  - [ ] `grep -n "update_card_whole" src/channels/lark.rs` returns ≥1 match (renamed method)
  - [ ] `grep -n "update_card_element" src/channels/lark.rs` returns ≥1 match (new method)
  - [ ] `grep -n "close_streaming" src/channels/lark.rs` returns ≥1 match (new method)
  - [ ] `grep -n "elements.*content.*content" src/channels/lark.rs` returns ≥1 match (element URL)
  - [ ] `stop_typing` still calls `update_card_whole` (not element-level)
  - [ ] `cargo test -- lark` passes

  **QA Scenarios**:
  ```
  Scenario: element-level update URL is correct
    Tool: Bash (grep)
    Steps:
      1. grep -n "elements.*content.*content" src/channels/lark.rs
      2. Verify URL pattern matches /cardkit/v1/cards/{card_id}/elements/content/content
    Expected Result: URL found in update_card_element method
    Evidence: .sisyphus/evidence/task-2-element-url.txt

  Scenario: stop_typing uses whole-card update (not element)
    Tool: Bash (grep)
    Steps:
      1. grep -A2 "stop_typing" src/channels/lark.rs | grep "update_card"
    Expected Result: calls update_card_whole, NOT update_card_element
    Evidence: .sisyphus/evidence/task-2-typing-whole.txt

  Scenario: close_streaming method exists with PATCH
    Tool: Bash (grep)
    Steps:
      1. grep -n "close_streaming" src/channels/lark.rs
      2. grep -n "\.patch" src/channels/lark.rs
    Expected Result: close_streaming method found, uses .patch() HTTP method
    Evidence: .sisyphus/evidence/task-2-close-streaming.txt
  ```

  **Commit**: YES
  - Message: `feat(lark): fix streaming card with element-level update and close_streaming`
  - Files: `src/channels/lark.rs`
  - Pre-commit: `cargo test -- lark`
- [ ] 3. Thread `message_id` into `ChannelMessage.thread_ts` for P2P messages
  **What to do**:
  - In `listen_ws()` (line 634), change `thread_ts: None` to:
    `thread_ts: if lark_msg.chat_type == "p2p" { Some(lark_msg.message_id.clone()) } else { None }`
  - In `parse_event_payload()` (line 1160), extract `message_id` from event JSON:
    `let message_id = event.pointer("/message/message_id").and_then(|v| v.as_str()).unwrap_or("");`
  - Change `thread_ts: None` at line 1167 to:
    `thread_ts: if chat_type == "p2p" { Some(message_id.to_string()) } else { None }`
  **Must NOT do**:
  - Do not modify `ChannelMessage` struct or `Channel` trait
  - Do not populate `thread_ts` for group chats
  - Do not change `listen_http()` webhook handler
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (parallel with Wave 1)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 4, Task 5
  - **Blocked By**: None
  **References**:
  - `src/channels/lark.rs:634-645` — `listen_ws()` ChannelMessage construction, `thread_ts: None`
  - `src/channels/lark.rs:1130-1168` — `parse_event_payload()` chat_type extraction + ChannelMessage construction
  - `src/channels/lark.rs:62-72` — `LarkMessage` struct has `message_id`, `chat_type`
  - `src/channels/traits.rs:14` — `ChannelMessage.thread_ts: Option<String>` (existing field)
  **Acceptance Criteria**:
  - [ ] `grep -n "thread_ts.*message_id" src/channels/lark.rs` returns ≥2 matches (ws + http paths)
  - [ ] `grep -n "p2p" src/channels/lark.rs` returns ≥2 matches (both paths gate on P2P)
  - [ ] `cargo test -- lark` passes
  **QA Scenarios**:
  ```
  Scenario: thread_ts populated for P2P in listen_ws
    Tool: Bash (grep)
    Steps:
      1. grep -n "thread_ts" src/channels/lark.rs | grep -v "//" | head -10
      2. Verify listen_ws path sets thread_ts = Some(message_id) for p2p
    Expected Result: thread_ts conditionally set based on chat_type == "p2p"
    Evidence: .sisyphus/evidence/task-3-thread-ts-ws.txt
  Scenario: thread_ts populated for P2P in parse_event_payload
    Tool: Bash (grep)
    Steps:
      1. grep -B2 -A2 "thread_ts" src/channels/lark.rs | grep -A2 "parse_event"
    Expected Result: parse_event_payload sets thread_ts for p2p
    Evidence: .sisyphus/evidence/task-3-thread-ts-http.txt
  ```
  **Commit**: NO (groups with Task 4)
- [ ] 4. Add `reply_text()` method and reply path in `send()`
  **What to do**:
  - Add URL helper: `fn reply_message_url(&self, message_id: &str) -> String { format!("{}/im/v1/messages/{message_id}/reply", self.api_base()) }`
  - Add `reply_text()` method that sends plain text via reply API:
    - `POST /im/v1/messages/{message_id}/reply`
    - Body: `{ "msg_type": "text", "content": "{\"text\": \"...\"}" }`
    - Follow token-refresh-retry pattern from `send_card_message()`
  - Modify `send()` (line 1350): at the top, check if `message.thread_ts.is_some()`
    - If yes: call `self.reply_text(thread_ts, &text).await` for the text portion
    - If no: continue with existing card-based send logic
    - Attachments still sent via existing attachment methods regardless
  **Must NOT do**:
  - Do not change `send_card_message()` — it sends card references, not text
  - Do not add reply support for group chats (thread_ts is only set for P2P by Task 3)
  - Do not convert markdown to rich text — send as-is
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (after Task 3)
  - **Blocks**: Task 5
  - **Blocked By**: Task 3
  **References**:
  - `src/channels/lark.rs:355-373` — URL helper methods (add `reply_message_url` here)
  - `src/channels/lark.rs:1350-1408` — `send()` method (add reply branch at top)
  - `src/channels/lark.rs:1243-1276` — `send_card_message()` token-refresh-retry pattern to follow
  - Feishu reply API: `POST /im/v1/messages/{message_id}/reply` with `{ msg_type, content }`
  **Acceptance Criteria**:
  - [ ] `grep -n "reply_message_url" src/channels/lark.rs` returns ≥1 match
  - [ ] `grep -n "reply_text" src/channels/lark.rs` returns ≥1 match
  - [ ] `grep -n "msg_type.*text" src/channels/lark.rs` returns ≥1 match
  - [ ] `grep -n "reply_in_thread" src/channels/lark.rs` returns ≥1 match
  - [ ] `cargo test -- lark` passes
  **QA Scenarios**:
  ```
  Scenario: reply_text sends plain text via reply API
    Tool: Bash (grep)
    Steps:
      1. grep -n "reply_message_url" src/channels/lark.rs
      2. grep -n "msg_type.*text" src/channels/lark.rs
      3. grep -n "reply_in_thread" src/channels/lark.rs
    Expected Result: reply URL helper, msg_type text, and reply_in_thread all present
    Evidence: .sisyphus/evidence/task-4-reply-text.txt
  Scenario: send() branches on thread_ts
    Tool: Bash (grep)
    Steps:
      1. grep -B1 -A3 "thread_ts" src/channels/lark.rs | grep -A3 "fn send"
    Expected Result: send() checks thread_ts and calls reply_text when set
    Evidence: .sisyphus/evidence/task-4-send-branch.txt
  ```
  **Commit**: YES
  - Message: `feat(lark): add plain text reply API for P2P messages`
  - Files: `src/channels/lark.rs`
  - Pre-commit: `cargo test -- lark`
- [ ] 5. Modify `channels/mod.rs`: stop-flag in draft_updater + always `send()` for final response
  **What to do**:
  This is the core architecture change. Three modifications to `channels/mod.rs`:

  **5a. Add stop-flag in draft_updater (lines 1673-1699)**:
  - After `DRAFT_CLEAR_SENTINEL` is received and `accumulated.clear()` runs, set a `card_closed = true` flag
  - When `card_closed` is true, ignore all subsequent deltas (don't call `update_draft()`)
  - This prevents final response text from being pushed into the streaming card
  - The flag is a simple `let mut card_closed = false;` before the while loop

  **5b. Change post-loop delivery (lines 1907-1929)**:
  - Current logic: if `draft_message_id.is_some()` → `finalize_draft(text)`, else → `send(text)`
  - New logic: if `draft_message_id.is_some()` → `finalize_draft("")` to close card, THEN `send(text)` with `thread_ts`
  - If `finalize_draft` fails → log warning, still try `send()`
  - If no draft → `send(text)` with `thread_ts` (existing behavior, unchanged)
  - This ensures final response ALWAYS goes through `send()` regardless of draft state

  **5c. Apply same pattern to error paths (lines 1986-1998, 2034-2047, 2076-2091)**:
  - Error messages: `finalize_draft("")` to close card, then `send(error_text)` with `thread_ts`
  - Follow same pattern as 5b for consistency
  **Must NOT do**:
  - Do not modify `agent/loop_.rs` — zero changes
  - Do not add channel-specific `if channel.name() == "lark"` checks — the logic is generic
  - Do not change `finalize_draft()` trait signature
  - Do not break Telegram's `finalize_draft()` — Telegram ignores empty text and still works
  - Do not restructure the entire post-loop delivery logic — minimal surgical changes only
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (after Wave 2)
  - **Blocks**: Task 6
  - **Blocked By**: Task 4
  **References**:
  - `src/channels/mod.rs:1673-1699` — draft_updater task with DRAFT_CLEAR_SENTINEL handling
  - `src/channels/mod.rs:1907-1929` — post-loop delivery: finalize_draft vs send
  - `src/channels/mod.rs:1986-1998` — context window error path with finalize_draft
  - `src/channels/mod.rs:2034-2047` — LLM error path with finalize_draft
  - `src/channels/mod.rs:2076-2091` — timeout error path with finalize_draft
  - `src/agent/loop_.rs:2300` — DRAFT_CLEAR_SENTINEL sent before final response
  - `src/channels/traits.rs:108-115` — `finalize_draft()` trait default (no-op)
  **Acceptance Criteria**:
  - [ ] Draft updater has `card_closed` flag that stops updates after CLEAR sentinel
  - [ ] Post-loop success path: `finalize_draft("")` then `send()` with `thread_ts`
  - [ ] Error paths follow same pattern: close card then send error via `send()`
  - [ ] `cargo test` passes (all channels, not just lark)
  - [ ] No `if channel.name() == "lark"` patterns in mod.rs
  **QA Scenarios**:
  ```
  Scenario: draft_updater stops after CLEAR sentinel
    Tool: Bash (grep)
    Steps:
      1. grep -n "card_closed" src/channels/mod.rs
      2. grep -n "DRAFT_CLEAR_SENTINEL" src/channels/mod.rs
    Expected Result: card_closed flag set after CLEAR, subsequent deltas ignored
    Evidence: .sisyphus/evidence/task-5-stop-flag.txt
  Scenario: finalize_draft called with empty text, then send() called
    Tool: Bash (grep)
    Steps:
      1. grep -B2 -A5 "finalize_draft" src/channels/mod.rs
      2. Verify finalize_draft is called with empty string
      3. Verify send() is called after finalize_draft for final response
    Expected Result: finalize_draft("") followed by send() with thread_ts
    Evidence: .sisyphus/evidence/task-5-flow-split.txt
  ```
  **Commit**: NO (groups with Task 6)
- [ ] 6. Re-enable `stream_mode` from config + `finalize_draft` closes card + unit tests
  **What to do**:
  **6a. Re-enable stream_mode (lines 284, 300, 316)**:
  - In `from_lark_channel_config()`, `from_lark_config()`, `from_feishu_config()`:
    Change `ch.stream_mode = StreamMode::Off;` to `ch.stream_mode = config.stream_mode.clone();`
  - Remove the comment about CardKit being broken
  **6b. Fix `finalize_draft()` (lines 1515-1550)**:
  - Current: calls `update_card()` with full card JSON (same as update_draft)
  - New: call `close_streaming(card_id, summary_text, sequence)` to close the streaming card
  - The `text` parameter from mod.rs will now be empty string (Task 5 change)
  - Use a short summary like "✅ Done" or the first 20 chars of text if non-empty
  - Then clean up tracking state (existing code)
  **6c. Add unit tests**:
  - Test that `from_feishu_config()` respects `stream_mode` from config
  - Test that `thread_ts` is populated for P2P in `parse_event_payload()`
  - Test that `reply_message_url()` constructs correct URL
  **Must NOT do**:
  - Do not change `start_typing()` or `stop_typing()`
  - Do not add new config schema keys
  - Do not change `cancel_draft()` behavior
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (after Task 5)
  - **Blocks**: F1-F3
  - **Blocked By**: Task 1, Task 2, Task 5
  **References**:
  - `src/channels/lark.rs:284,300,316` — three `from_*config()` methods with `StreamMode::Off`
  - `src/channels/lark.rs:1515-1550` — `finalize_draft()` current implementation
  - `src/channels/lark.rs:1939-2773` — test module for adding new tests
  - `src/config/schema.rs` — `StreamMode` enum and config fields
  **Acceptance Criteria**:
  - [ ] `grep -n "StreamMode::Off" src/channels/lark.rs` returns 0 matches in `from_*config()` methods
  - [ ] `grep -n "close_streaming" src/channels/lark.rs` found in `finalize_draft()`
  - [ ] `cargo test -- lark` passes with new tests
  - [ ] `cargo clippy --all-targets -- -D warnings` clean
  **QA Scenarios**:
  ```
  Scenario: stream_mode read from config
    Tool: Bash (grep)
    Steps:
      1. grep -n "StreamMode::Off" src/channels/lark.rs
    Expected Result: 0 matches in from_*config() methods
    Evidence: .sisyphus/evidence/task-6-stream-mode.txt
  Scenario: finalize_draft calls close_streaming
    Tool: Bash (grep)
    Steps:
      1. grep -A10 "fn finalize_draft" src/channels/lark.rs | grep "close_streaming"
    Expected Result: close_streaming called inside finalize_draft
    Evidence: .sisyphus/evidence/task-6-finalize-close.txt
  ```
  **Commit**: YES
  - Message: `feat(channels): split tool notification card from final response delivery`
  - Files: `src/channels/lark.rs`, `src/channels/mod.rs`
  - Pre-commit: `cargo test && cargo clippy --all-targets -- -D warnings`
---

## Final Verification Wave

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists. For each "Must NOT Have": search codebase for forbidden patterns. Check evidence files exist. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo test` + `cargo clippy --all-targets -- -D warnings`. Review all changed files for: `as any`/`@ts-ignore` equivalents, empty catches, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | VERDICT`

- [ ] F3. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Detect cross-task contamination. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | VERDICT`

---

## Commit Strategy

- **Wave 1-2**: `feat(lark): fix streaming card element-level update and add reply API` — `src/channels/lark.rs`
- **Wave 3**: `feat(channels): split tool notification card from final response delivery` — `src/channels/mod.rs`, `src/channels/lark.rs`

---

## Success Criteria

### Verification Commands
```bash
cargo test                                          # All tests pass
cargo clippy --all-targets -- -D warnings           # No warnings
grep -n "StreamMode::Off" src/channels/lark.rs      # 0 matches in from_*config()
grep -n "reply_in_thread" src/channels/lark.rs      # ≥1 match
grep -n "element_id" src/channels/lark.rs           # ≥1 match
grep -n "streaming_mode" src/channels/lark.rs       # ≥1 match
grep -n "msg_type.*text" src/channels/lark.rs       # ≥1 match
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] Telegram draft lifecycle preserved
- [ ] Slack/Discord no-op paths unbroken
