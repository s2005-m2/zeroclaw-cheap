# Lark Channel Feature Superset + Feishu Docs Bidirectional Sync

## TL;DR

> **Quick Summary**: 将飞书频道升级为 Telegram 功能超集，补齐 cron 推送（含 hook）、CardKit 流式更新、typing 指示器、文件收发，并新增飞书文档双向配置同步。
> 
> **Deliverables**:
> - Lark cron delivery（含 on_cron_delivery modifying hook）
> - CardKit 卡片实体流式更新
> - Typing 指示器（CardKit "处理中" 卡片）
> - 文件/文档/音频/视频发送
> - 接收用户附件
> - 飞书文档双向配置同步（含 on_docs_sync_notify modifying hook）
> - 权限与事件订阅设置指南
> 
> **Estimated Effort**: Large
> **Parallel Execution**: YES - 3 waves
> **Critical Path**: Task 4 → Task 5 → Task 7 → Task 8

---

## Context

### Original Request
用户以飞书为主要渠道，要求飞书功能成为 Telegram 超集。6 项需求 + 推送逻辑支持 hook。

### Interview Summary
- 流式更新：飞书消息编辑有 ~20-30 次隐性上限，必须用 CardKit 卡片实体
- Typing：飞书无原生 API，用 CardKit 替代
- Hook：新增 on_cron_delivery + on_docs_sync_notify 两个 modifying hook（可修改/取消）
- 文档同步：完全双向实时，代码块形式，需写权限指南

### Metis Review
- CardKit 降级：失败时回退到现有 interactive card
- 安全：禁止远程修改 [security]/[gateway]/[autonomy]
- 文件限制：20MB
- Feature flag：docs sync 独立 feishu-docs-sync flag
- Hook：cron delivery 当前绕过 hook 系统，需修复

---

## Work Objectives

### Core Objective
飞书频道成为 Telegram 功能超集 + 飞书文档双向配置同步 + 推送 hook 支持。

### Must Have
- CardKit 流式更新（降级到 interactive card）
- 文件收发（file/audio/video/media）
- Cron delivery 支持 lark/feishu + on_cron_delivery hook
- 飞书文档双向同步 + on_docs_sync_notify hook
- 设置指南文档

### Must NOT Have (Guardrails)
- 不修改 Channel trait
- 不添加语音转文字、/bind、/model 命令
- 不构建通用飞书文档编辑器
- 文档同步不写配置列表外的文件
- 文档同步不修改 [security]/[gateway]/[autonomy]
- 消息功能不引入新依赖
- 文件下载不超过 20MB

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

- **Infrastructure**: YES (cargo test)
- **Automated tests**: YES (Tests-after)
- **Framework**: cargo test
- **QA**: Agent-executed, evidence to `.sisyphus/evidence/`

---

## Execution Strategy

```
Wave 1 (Start Immediately — 5 parallel):
├── Task 1: on_cron_delivery + on_docs_sync_notify hooks [quick]
├── Task 2: Cron delivery — add lark/feishu with hook [quick]
├── Task 3: File/document sending [unspecified-high]
├── Task 4: CardKit draft streaming [deep]
└── Task 5: File/attachment receiving [unspecified-high]

Wave 1b (After Task 1 + Task 4):
└── Task 6: Typing indicator [quick]

Wave 2 (After Wave 1 — docs sync):
└── Task 7: Feishu Docs bidirectional config sync [deep]

Wave 3 (After Wave 2 — docs, 2 parallel):
├── Task 8: Feishu Docs sync setup guide [writing]
└── Task 9: Update channels-reference.md [writing]

Wave FINAL (After ALL — 4 parallel):
├── F1: Plan compliance audit [oracle]
├── F2: Code quality review [unspecified-high]
├── F3: Real manual QA [unspecified-high]
└── F4: Scope fidelity check [deep]

Critical Path: Task 1 → Task 2, Task 4 → Task 6 → Task 7 → Task 8
Max Concurrent: 5 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 2, 7 | 1 |
| 2 | 1 | F1-F4 | 1 |
| 3 | — | 9, F1-F4 | 1 |
| 4 | — | 6, F1-F4 | 1 |
| 5 | — | 9, F1-F4 | 1 |
| 6 | 1, 4 | 7, F1-F4 | 1b |
| 7 | 1-6 | 8, F1-F4 | 2 |
| 8 | 7 | F1-F4 | 3 |
| 9 | 1-6 | F1-F4 | 3 |

---

## TODOs


- [ ] 1. Add on_cron_delivery and on_docs_sync_notify modifying hooks

  **What to do**:
  - Add `on_cron_delivery` method to `HookHandler` trait in `src/hooks/traits.rs` — modifying hook with signature `(source: String, channel: String, recipient: String, content: String) -> HookResult<(String, String, String, String)>`. Source identifies the trigger (e.g. cron job id).
  - Add `on_docs_sync_notify` method to `HookHandler` trait — modifying hook with signature `(file_path: String, channel: String, recipient: String, content: String) -> HookResult<(String, String, String, String)>`. file_path identifies which synced file changed.
  - Add `run_on_cron_delivery` and `run_on_docs_sync_notify` sequential dispatchers in `src/hooks/runner.rs` following the exact pattern of `run_on_message_sending` (lines 277-314).
  - Add `fire_delivery_sent` void hook for post-delivery notification (or reuse existing `fire_message_sent`).
  - Add unit tests: modifying hook can alter delivery target, cancel hook blocks delivery.

  **Must NOT do**:
  - Do not modify existing hook signatures
  - Do not add hooks that are not modifying (these must be modifying per user requirement)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4, 5)
  - **Blocks**: Tasks 2, 7
  - **Blocked By**: None

  **References**:
  - `src/hooks/traits.rs:71-78` — `on_message_sending` pattern to follow exactly
  - `src/hooks/runner.rs:277-314` — `run_on_message_sending` sequential dispatcher pattern
  - `src/hooks/runner.rs:101-108` — `fire_message_sent` void hook pattern for post-delivery

  **Acceptance Criteria**:
  - [ ] `cargo test --features channel-lark -- hooks` passes
  - [ ] New hook methods have default no-op implementations
  - [ ] Modifying hooks can alter all 4 parameters (source/file_path, channel, recipient, content)
  - [ ] Cancel hook returns HookResult::Cancel and blocks delivery

  **QA Scenarios**:
  ```
  Scenario: Hook can modify cron delivery target
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- hooks::tests::on_cron_delivery_modifies_target
      2. Assert test passes — hook changes recipient from "chat_a" to "chat_b"
    Expected Result: Test passes, modified recipient is "chat_b"
    Evidence: .sisyphus/evidence/task-1-hook-modify.txt

  Scenario: Hook can cancel cron delivery
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- hooks::tests::on_cron_delivery_cancel
      2. Assert test passes — hook returns Cancel, delivery is blocked
    Expected Result: Test passes, HookResult::Cancel returned
    Evidence: .sisyphus/evidence/task-1-hook-cancel.txt
  ```

  **Commit**: YES
  - Message: `feat(hooks): add on_cron_delivery and on_docs_sync_notify modifying hooks`
  - Files: `src/hooks/traits.rs`, `src/hooks/runner.rs`
  - Pre-commit: `cargo test --features channel-lark -- hooks`

- [ ] 2. Add Lark/Feishu to cron delivery with hook support

  **What to do**:
  - Add `"lark"` and `"feishu"` match arms in `deliver_if_configured()` in `src/cron/scheduler.rs` (after line 357). Construct `LarkChannel` from config using `from_lark_config` / `from_feishu_config`. Gate behind `#[cfg(feature = "channel-lark")]`.
  - Before calling `channel.send()`, run `hooks.run_on_cron_delivery(job_id, channel_name, target, output)`. If hook returns Cancel, skip delivery and log warning.
  - After successful send, call `hooks.fire_message_sent(channel_name, target, output)` to reuse existing void hook.
  - Pass `HookRunner` reference into `deliver_if_configured()` — update function signature to accept `Option<&HookRunner>`.
  - Add unit test for lark/feishu match arms.

  **Must NOT do**:
  - Do not change behavior of existing telegram/discord/slack/mattermost delivery paths
  - Do not add hook support to existing channels in this task (separate concern)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Task 1)
  - **Parallel Group**: Wave 1 (starts after Task 1 hook trait is ready)
  - **Blocks**: F1-F4
  - **Blocked By**: Task 1

  **References**:
  - `src/cron/scheduler.rs:284-361` — `deliver_if_configured()` with existing telegram/discord/slack/mattermost arms
  - `src/cron/scheduler.rs:299-312` — Telegram delivery pattern to follow exactly
  - `src/channels/lark.rs:1043-1080` — `LarkChannel::send()` method
  - `src/channels/mod.rs:2864-2892` — How LarkChannel is constructed from config

  **Acceptance Criteria**:
  - [ ] `cargo test --features channel-lark -- scheduler` passes
  - [ ] "lark" and "feishu" no longer trigger "unsupported delivery channel" error
  - [ ] Hook is called before send, Cancel blocks delivery

  **QA Scenarios**:
  ```
  Scenario: Cron delivery compiles for lark channel
    Tool: Bash (cargo build)
    Steps:
      1. cargo build --features channel-lark
      2. Assert exit code 0
    Expected Result: Build succeeds with lark delivery support
    Evidence: .sisyphus/evidence/task-2-build.txt

  Scenario: Lark delivery match arm works
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- scheduler::tests
      2. Assert new lark/feishu delivery tests pass
    Expected Result: All scheduler tests pass
    Evidence: .sisyphus/evidence/task-2-test.txt
  ```

  **Commit**: YES
  - Message: `feat(cron): add lark/feishu delivery with hook support`
  - Files: `src/cron/scheduler.rs`
  - Pre-commit: `cargo test --features channel-lark -- scheduler`


- [ ] 3. Implement file/document sending for Lark
  **What to do**:
  - Add `upload_file()` method to `LarkChannel` — `POST /im/v1/files` (multipart/form-data) with file_type mapping: pdf/doc/xls/ppt/opus/mp4/stream. Add `LARK_MAX_FILE_UPLOAD_BYTES` constant (20MB).
  - Add `send_file_msg()`, `send_audio_msg()`, `send_media_msg()` methods using `POST /im/v1/messages` with msg_type=file/audio/media.
  - Extend existing `parse_lark_image_markers()` to `parse_lark_attachment_markers()` — handle `[DOCUMENT:path]`, `[AUDIO:path]`, `[VIDEO:path]` markers in outgoing messages.
  - Update `send()` to dispatch attachment markers to appropriate send methods.
  - Add file_type resolver: extension → feishu file_type (pdf/doc/xls/ppt/opus/mp4/stream).
  - Add unit tests for marker parsing, URL construction, file_type mapping.
  **Must NOT do**:
  - Do not add voice transcription
  - Do not modify Channel trait
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4, 5)
  - **Blocks**: Task 9, F1-F4
  - **Blocked By**: None
  **References**:
  - `src/channels/lark.rs:1335-1370` — existing `parse_lark_image_markers()` to extend
  - `src/channels/lark.rs:801-830` — existing `upload_image()` pattern for multipart upload
  - `src/channels/lark.rs:330-334` — URL helper pattern (`send_message_url`, `upload_image_url`)
  - `src/channels/telegram.rs:1659-1729` — Telegram file sending pattern (multipart form)
  - `src/channels/telegram.rs:248-280` — `parse_attachment_markers()` pattern to follow
  - Feishu API: `POST /im/v1/files` (file_type: pdf|doc|xls|ppt|opus|mp4|stream)
  - Feishu API: `POST /im/v1/messages` (msg_type: file|audio|media)
  **Acceptance Criteria**:
  - [ ] `cargo test --features channel-lark -- lark::tests` passes
  - [ ] Marker parsing handles [DOCUMENT:], [AUDIO:], [VIDEO:] correctly
  - [ ] File type resolver maps extensions correctly
  **QA Scenarios**:
  ```
  Scenario: Attachment marker parsing
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::parse_lark_attachment_markers
      2. Assert [DOCUMENT:/tmp/a.pdf] extracts as ("document", "/tmp/a.pdf")
      3. Assert [AUDIO:/tmp/b.opus] extracts as ("audio", "/tmp/b.opus")
    Expected Result: All marker types parsed correctly
    Evidence: .sisyphus/evidence/task-3-markers.txt
  Scenario: File type resolver
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::resolve_feishu_file_type
      2. Assert .pdf→"pdf", .docx→"doc", .mp4→"mp4", .txt→"stream"
    Expected Result: All extensions map correctly
    Evidence: .sisyphus/evidence/task-3-filetype.txt
  ```
  **Commit**: YES
  - Message: `feat(lark): implement file/document/audio/video sending`
  - Files: `src/channels/lark.rs`
  - Pre-commit: `cargo test --features channel-lark -- lark::tests`

- [ ] 4. Implement CardKit draft streaming for Lark
  **What to do**:
  - Add CardKit API methods to `LarkChannel`: `create_card()` (POST /cardkit/v1/card), `update_card()` (PUT /cardkit/v1/card/{id} with sequence++), `send_card_message()` (POST /im/v1/messages with msg_type=interactive referencing card_id).
  - Add `cardkit_url()` helper following `LarkPlatform` pattern for base URL.
  - Implement Channel trait draft methods: `supports_draft_updates()` → true when `stream_mode` enabled, `send_draft()` → create card + send card message + return card_id, `update_draft()` → update card content with throttle (500ms), `finalize_draft()` → final card update, `cancel_draft()` → delete card message.
  - Add `stream_mode` field to `LarkConfig`/`FeishuConfig` in `src/config/schema.rs`.
  - Add `card_sequence: Mutex<HashMap<String, u64>>` for sequence tracking.
  - Add `draft_update_interval_ms` field (default 500).
  - Fallback: if CardKit API returns error, fall back to existing interactive card send path.
  - Card content format: JSON 2.0 schema with markdown body element.
  **Must NOT do**:
  - Do not use message edit API (PATCH /im/v1/messages) — has ~20-30 edit limit
  - Do not modify Channel trait in traits.rs
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3, 5)
  - **Blocks**: Task 6, F1-F4
  - **Blocked By**: None
  **References**:
  - `src/channels/telegram.rs:2146-2400` — Telegram draft streaming implementation pattern
  - `src/channels/lark.rs:1043-1080` — existing `send()` method (fallback target)
  - `src/channels/lark.rs:330-334` — URL helper pattern
  - `src/config/schema.rs:3120+` — LarkConfig struct to extend (NOT line 2780 which is MattermostConfig)
  - CardKit API: POST /cardkit/v1/card (create), PUT /cardkit/v1/card/{id} (update)
  - Card JSON 2.0: `{"schema":"2.0","body":{"elements":[{"tag":"markdown","content":"..."}]}}`
  **Acceptance Criteria**:
  - [ ] `cargo test --features channel-lark -- lark::tests` passes
  - [ ] `supports_draft_updates()` returns true when stream_mode configured
  - [ ] Sequence number increments per card_id
  - [ ] Throttle enforced at 500ms minimum between updates
  **QA Scenarios**:
  ```
  Scenario: CardKit URL construction
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::cardkit_url
      2. Assert Feishu → open.feishu.cn/open-apis/cardkit/v1/card
      3. Assert Lark → open.larksuite.com/open-apis/cardkit/v1/card
    Expected Result: Both platform URLs correct
    Evidence: .sisyphus/evidence/task-4-cardkit-url.txt
  Scenario: Streaming sequence tracking
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::card_sequence_increments
      2. Assert sequence goes 1→2→3 for same card_id
    Expected Result: Monotonically increasing sequence
    Evidence: .sisyphus/evidence/task-4-sequence.txt
  ```
  **Commit**: YES
  - Message: `feat(lark): implement CardKit draft streaming`
  - Files: `src/channels/lark.rs`, `src/config/schema.rs`
  - Pre-commit: `cargo test --features channel-lark -- lark::tests`

- [ ] 5. Implement file/attachment receiving for Lark
  **What to do**:
  - Extend `parse_event_payload()` in lark.rs to handle `file`, `audio`, `media` message types from `im.message.receive_v1` events. Extract `file_key` and `file_name` from message content JSON.
  - Add `download_file()` method: `GET /im/v1/messages/{message_id}/resources/{file_key}?type=file`. For images use `type=image`, for audio/video/file use `type=file`.
  - Save downloaded files to `workspace/lark_files/`. Add `workspace_dir` field to `LarkChannel` (follow Telegram's pattern).
  - Format received attachments as `[DOCUMENT:path]` / `[IMAGE:path]` / `[AUDIO:path]` markers in `ChannelMessage.content` so the agent loop can process them.
  - Add `LARK_MAX_FILE_DOWNLOAD_BYTES` constant (20MB). Reject files exceeding limit.
  - Add unit tests for message type parsing and marker generation.
  **Must NOT do**:
  - Do not add voice transcription
  - Do not follow symlinks when saving files
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3, 4)
  - **Blocks**: Task 9, F1-F4
  - **Blocked By**: None
  **References**:
  - `src/channels/lark.rs:600-730` — existing `parse_event_payload()` for text/image messages
  - `src/channels/lark.rs:869-890` — existing `send_lark_image()` download pattern
  - `src/channels/telegram.rs:863-1037` — Telegram attachment receiving pattern
  - Feishu API: `GET /im/v1/messages/{id}/resources/{key}?type=image|file`
  **Acceptance Criteria**:
  - [ ] `cargo test --features channel-lark -- lark::tests` passes
  - [ ] file/audio/media message types parsed correctly
  - [ ] file_key extracted from content JSON
  **QA Scenarios**:
  ```
  Scenario: Parse file message type
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::parse_file_message
      2. Assert message_type="file" extracts file_key and file_name
    Expected Result: file_key and file_name correctly extracted
    Evidence: .sisyphus/evidence/task-5-parse-file.txt
  Scenario: Parse audio message type
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::parse_audio_message
      2. Assert message_type="audio" extracts file_key with type=file download
    Expected Result: audio file_key extracted, download type is "file"
    Evidence: .sisyphus/evidence/task-5-parse-audio.txt
  ```
  **Commit**: YES
  - Message: `feat(lark): implement attachment receiving and download`
  - Files: `src/channels/lark.rs`
  - Pre-commit: `cargo test --features channel-lark -- lark::tests`
- [ ] 6. Implement typing indicator for Lark via CardKit
  **What to do**:
  - Implement `start_typing()` on `LarkChannel`: create a minimal CardKit card with "正在处理..." content, send as card message, store card_id in `typing_card_ids: Mutex<HashMap<String, String>>` keyed by recipient.
  - Implement `stop_typing()`: look up card_id for recipient, update card to empty/remove. If CardKit unavailable, no-op gracefully.
  - Reuse CardKit methods from Task 4 (`create_card`, `send_card_message`).
  - The typing card will be replaced by the actual draft card when `send_draft()` is called, so `stop_typing` should also clean up the tracking map.
  **Must NOT do**:
  - Do not add a separate typing API (飞书没有)
  - Do not block if CardKit is unavailable — graceful no-op
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1b
  - **Blocks**: Task 7, F1-F4
  - **Blocked By**: Task 1 (hooks), Task 4 (CardKit methods)
  **References**:
  - `src/channels/telegram.rs:2563-2600` — Telegram typing implementation pattern
  - `src/channels/lark.rs` — CardKit methods added in Task 4
  **Acceptance Criteria**:
  - [ ] `cargo test --features channel-lark -- lark::tests` passes
  - [ ] start_typing/stop_typing don't panic when CardKit unavailable
  **QA Scenarios**:
  ```
  Scenario: Typing lifecycle
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark -- lark::tests::typing_lifecycle
      2. Assert start_typing stores card_id, stop_typing clears it
    Expected Result: typing_card_ids map correctly managed
    Evidence: .sisyphus/evidence/task-6-typing.txt
  ```
  **Commit**: YES
  - Message: `feat(lark): implement typing indicator via CardKit`
  - Files: `src/channels/lark.rs`
  - Pre-commit: `cargo test --features channel-lark -- lark::tests`
- [ ] 7. Implement Feishu Docs bidirectional config sync module
  **What to do**:
  - Create `src/docs_sync/` module behind `feishu-docs-sync` feature flag. Submodules: `mod.rs`, `client.rs` (Feishu Docs API), `sync.rs` (sync engine), `watcher.rs` (local file watch).
  - `client.rs`: Feishu Docs API client — `get_raw_content()` (GET /docx/v1/documents/{id}/raw_content), `get_blocks()`, `batch_update_blocks()` (PATCH, 3/s rate limit), `create_document()`. Share tenant_access_token with LarkChannel.
  - `sync.rs`: Bidirectional sync engine — (a) Parse code blocks from Feishu doc to extract file contents, (b) Serialize local files as code blocks into Feishu doc, (c) Conflict resolution: revision_id comparison + last-write-wins + warning log, (d) Security: reject remote changes to `[security]`/`[gateway]`/`[autonomy]` sections in config.toml, (e) Rate limiter (3 req/s writes).
  - `watcher.rs`: Local file watcher using `notify` crate — watch configured files, debounce 500ms, trigger remote push on change.
  - Remote→Local: subscribe to `drive.file.edit_v1` event via existing WS/webhook, trigger local pull on change.
  - Before sending sync notifications, run `hooks.run_on_docs_sync_notify()`. If Cancel, skip notification.
  - Add `[docs_sync]` config section to `src/config/schema.rs`: `enabled` (default false), `document_id`, `sync_files` (vec of paths), `sync_interval_secs`, `auto_create_doc` (bool).
  - Default sync_files: `["config.toml", "IDENTITY.md", "SOUL.md", "USER.md", "AGENTS.md"]`.
  - ZeroClaw can dynamically add files via agent tool call or config update.
  **Must NOT do**:
  - Do not write to files outside configured sync_files list
  - Do not follow symlinks
  - Do not modify [security]/[gateway]/[autonomy] from remote
  - Do not build a general-purpose Feishu doc editor
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (after all Wave 1 tasks)
  - **Blocks**: Task 8, F1-F4
  - **Blocked By**: Tasks 1-6
  **References**:
  - `src/config/schema.rs:1895-1916` — HooksConfig pattern for new config section
  - `src/channels/lark.rs:100-200` — tenant_access_token caching pattern to share
  - `src/heartbeat/engine.rs` — periodic engine pattern (interval loop + file reading)
  - Feishu Docs API: GET /docx/v1/documents/{id}/raw_content
  - Feishu Docs API: PATCH /docx/v1/documents/{id}/blocks/batch_update (3/s limit)
  - Feishu Event: drive.file.edit_v1
  **Acceptance Criteria**:
  - [ ] `cargo build --features channel-lark,feishu-docs-sync` succeeds
  - [ ] `cargo test --features channel-lark,feishu-docs-sync -- docs_sync` passes
  - [ ] Config parsing for [docs_sync] section works
  - [ ] Security rejection test: remote [security] changes blocked
  **QA Scenarios**:
  ```
  Scenario: Config parsing for docs_sync
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark,feishu-docs-sync -- docs_sync::tests::config_parsing
      2. Assert enabled=false by default, sync_files has 5 defaults
    Expected Result: Config defaults correct
    Evidence: .sisyphus/evidence/task-7-config.txt
  Scenario: Security rejection
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark,feishu-docs-sync -- docs_sync::tests::reject_security_section
      2. Assert remote TOML with [security] changes is rejected
    Expected Result: SecurityRejection error returned
    Evidence: .sisyphus/evidence/task-7-security.txt
  Scenario: Code block serialization roundtrip
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --features channel-lark,feishu-docs-sync -- docs_sync::tests::roundtrip
      2. Assert local file → code block → local file produces identical content
    Expected Result: Lossless roundtrip
    Evidence: .sisyphus/evidence/task-7-roundtrip.txt
  ```
  **Commit**: YES
  - Message: `feat(docs-sync): implement feishu docs bidirectional config sync`
  - Files: `src/docs_sync/*`, `src/config/schema.rs`, `src/lib.rs`, `Cargo.toml`
  - Pre-commit: `cargo test --features channel-lark,feishu-docs-sync`
- [ ] 8. Write Feishu Docs sync setup guide
  **What to do**:
  - Create `docs/feishu-docs-sync-guide.md` with: (a) Required Feishu app permissions (docx:document, docx:document:readonly, drive:drive, im:message, im:message:send_as_bot), (b) Event subscription setup (drive.file.edit_v1 via WS long-connection or webhook), (c) Config.toml example for `[docs_sync]` section, (d) Troubleshooting (rate limits, permission errors, conflict resolution), (e) Security considerations (which config sections are protected).
  - Update `docs/SUMMARY.md` with link to new guide.
  - Note i18n follow-up needed per AGENTS.md §4.1.
  **Must NOT do**:
  - Do not write guides for features not yet implemented
  **Recommended Agent Profile**:
  - **Category**: `writing`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 9)
  - **Parallel Group**: Wave 3
  - **Blocks**: F1-F4
  - **Blocked By**: Task 7
  **References**:
  - `docs/channels-reference.md` — existing channel docs format
  - `docs/SUMMARY.md` — TOC to update
  - `src/config/schema.rs` — [docs_sync] config fields added in Task 7
  **Acceptance Criteria**:
  - [ ] Markdown lint passes
  - [ ] All internal links resolve
  - [ ] docs/SUMMARY.md includes new entry
  **QA Scenarios**:
  ```
  Scenario: Guide completeness
    Tool: Bash
    Steps:
      1. Check docs/feishu-docs-sync-guide.md exists
      2. Grep for "docx:document" permission listed
      3. Grep for "drive.file.edit_v1" event listed
      4. Grep for "[docs_sync]" config example
    Expected Result: All sections present
    Evidence: .sisyphus/evidence/task-8-guide.txt
  ```
  **Commit**: YES
  - Message: `docs: add feishu docs sync setup guide`
  - Files: `docs/feishu-docs-sync-guide.md`, `docs/SUMMARY.md`
- [ ] 9. Update channels-reference.md with Lark feature parity
  **What to do**:
  - Update `docs/channels-reference.md` to document new Lark capabilities: file sending/receiving, CardKit streaming, typing indicator, cron delivery.
  - Add config examples for `stream_mode`, `workspace_dir`.
  - Keep consistent with existing channel documentation format.
  **Recommended Agent Profile**:
  - **Category**: `writing`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 8)
  - **Parallel Group**: Wave 3
  - **Blocks**: F1-F4
  - **Blocked By**: Tasks 1-6
  **References**:
  - `docs/channels-reference.md` — existing format to follow
  - `src/config/schema.rs` — new config fields
  **Acceptance Criteria**:
  - [ ] Markdown lint passes
  - [ ] All config keys documented match actual schema
  **QA Scenarios**:
  ```
  Scenario: Doc completeness
    Tool: Bash
    Steps:
      1. Grep channels-reference.md for "stream_mode"
      2. Grep for "CardKit"
      3. Grep for "file" sending section
    Expected Result: All new features documented
    Evidence: .sisyphus/evidence/task-9-doc.txt
  ```
  **Commit**: YES
  - Message: `docs: update channels-reference with lark feature parity`
  - Files: `docs/channels-reference.md`
## Final Verification Wave

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Verify all Must Have implemented, all Must NOT Have absent. Check evidence files.

- [ ] F2. **Code Quality Review** — `unspecified-high`
  `cargo fmt --check` + `cargo clippy --features channel-lark,feishu-docs-sync -D warnings` + `cargo test`.

- [ ] F3. **Real Manual QA** — `unspecified-high`
  Execute every QA scenario. Save to `.sisyphus/evidence/final-qa/`.

- [ ] F4. **Scope Fidelity Check** — `deep`
  Verify 1:1 spec-to-implementation. No scope creep.

---

## Commit Strategy

- T1: `feat(hooks): add on_cron_delivery and on_docs_sync_notify modifying hooks`
- T2: `feat(cron): add lark/feishu delivery with hook support`
- T3: `feat(lark): implement file/document/audio/video sending`
- T4: `feat(lark): implement CardKit draft streaming`
- T5: `feat(lark): implement attachment receiving and download`
- T6: `feat(lark): implement typing indicator via CardKit`
- T7: `feat(docs-sync): implement feishu docs bidirectional config sync`
- T8: `docs: add feishu docs sync setup guide`
- T9: `docs: update channels-reference with lark feature parity`

---

## Success Criteria

```bash
cargo build --features channel-lark,feishu-docs-sync
cargo clippy --features channel-lark,feishu-docs-sync --all-targets -- -D warnings
cargo test --features channel-lark,feishu-docs-sync
```

- [ ] All Must Have present
- [ ] All Must NOT Have absent
- [ ] All tests pass
- [ ] Hook system: on_cron_delivery + on_docs_sync_notify functional
