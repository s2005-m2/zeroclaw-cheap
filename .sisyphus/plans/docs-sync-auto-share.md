# Docs Sync Auto-Share: 飞书文档自动分享给聊天用户

## TL;DR

> **Quick Summary**: 当 ZeroClaw 首次收到飞书用户消息时，自动将 docs_sync 同步的所有文档分享给该用户（通过 open_id），实现文档对聊天用户"可见"。
> 
> **Deliverables**:
> - `FeishuDocsClient` 新增 `add_permission_member` 方法
> - LarkChannel 提取并传递 `open_id` 到 docs_sync
> - docs_sync worker 接收 open_id 后自动分享所有已同步文档
> - 持久化已分享状态，避免重启后重复调用
> 
> **Estimated Effort**: Short
> **Parallel Execution**: YES - 2 waves
> **Critical Path**: Task 1 → Task 2 → Task 3 → Task 4

---

## Context

### Original Request
文件和飞书双向绑定已实现（docs_sync 模块）。用户希望在 ZeroClaw 首次接收到飞书消息后，自动将同步文档分享给该用户，使文档对用户可见。单用户场景，无需多用户并发。

### Interview Summary
**Key Discussions**:
- 飞书权限 API 已验证：`POST /open-apis/drive/v1/permissions/:token/members`，`tenant_access_token` 可用
- `batch_create` 是对单文档加多用户，不适用当前场景（单用户多文档）
- 4 个文档逐个调用 `create` 即可，约 1.5 秒完成
- 应用需额外开启 `drive:drive:permission` 权限范围

**Research Findings**:
- 飞书官方 Go SDK 确认 API 路径：`/open-apis/drive/v1/permissions/:token/members`
- `BaseMember` 结构：`member_type: "openid"`, `member_id: "<open_id>"`, `perm: "view"`
- Query param `type=docx` 对应新版文档，`need_notification=false` 静默分享

### Metis Review
**Identified Gaps** (addressed):
- `open_id` 在 `ChannelMessage` 构建时被丢弃（`sender` 存的是 `chat_id`）→ 需要在 LarkChannel 内部直接触发分享，不经过 `ChannelMessage`
- 重启后已分享状态丢失 → 持久化到 lock 文件
- docs_sync 未启用时不应触发分享逻辑 → 条件检查

---

## Work Objectives

### Core Objective
首次收到飞书用户消息时，自动将 docs_sync 所有已同步文档的阅读权限授予该用户。

### Concrete Deliverables
- `src/docs_sync/client.rs`: 新增 `add_permission_member` 方法
- `src/docs_sync/worker.rs` 或 `src/docs_sync/mod.rs`: 暴露共享状态（lock map 中的 doc_ids）和分享触发入口
- `src/channels/lark.rs`: 在消息处理流程中检测新用户并触发文档分享
- 持久化文件记录已分享的 open_id

### Definition of Done
- [ ] 新用户首次发消息后，所有 docs_sync 文档对该用户可见
- [ ] 重启后不重复分享已分享用户
- [ ] 分享失败不阻塞消息处理流程
- [ ] `cargo clippy --all-targets -- -D warnings` 通过
- [ ] `cargo test` 通过

### Must Have
- 静默分享（`need_notification=false`）
- 分享失败仅 warn 日志，不阻塞消息流
- 持久化已分享状态
- 仅在 docs_sync 启用时触发

### Must NOT Have (Guardrails)
- 不修改 `ChannelMessage` 结构体（跨 channel 影响太大）
- 不引入新的 crate 依赖
- 不实现多用户并发追踪/注册系统
- 不实现 batch_create（单用户场景无需）
- 不添加新的 config 字段（复用现有 docs_sync 配置）
- 不在分享失败时 panic 或中断消息处理

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (cargo test)
- **Automated tests**: Tests-after（对新增的 client 方法和状态持久化逻辑）
- **Framework**: cargo test

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **API/Backend**: Use Bash (cargo test / cargo clippy) — compile, lint, test
- **Integration**: Use Bash (cargo build --release) — verify full build

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — independent):
├── Task 1: FeishuDocsClient 新增 add_permission_member [quick]
├── Task 2: 共享状态模块 — 暴露 doc_ids + 已分享用户持久化 [quick]

Wave 2 (Integration — depends on Wave 1):
├── Task 3: LarkChannel 集成 — 检测新用户并触发分享 [unspecified-high]

Wave 3 (Verification):
├── Task 4: 全量编译验证 + clippy + test [quick]

Wave FINAL (Review):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Scope fidelity check (deep)
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1    | —         | 3      |
| 2    | —         | 3      |
| 3    | 1, 2      | 4      |
| 4    | 3         | F1-F3  |
| F1-F3| 4         | —      |

### Agent Dispatch Summary

- **Wave 1**: 2 tasks — T1 → `quick`, T2 → `quick`
- **Wave 2**: 1 task — T3 → `unspecified-high`
- **Wave 3**: 1 task — T4 → `quick`
- **FINAL**: 3 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `deep`

---

## TODOs


- [x] 1. FeishuDocsClient 新增 `add_permission_member` 方法

  **What to do**:
  - 在 `src/docs_sync/client.rs` 的 `FeishuDocsClient` impl 块中新增 `pub async fn add_permission_member(&self, document_id: &str, open_id: &str, perm: &str) -> Result<()>`
  - 调用 `POST {FEISHU_BASE_URL}/drive/v1/permissions/{document_id}/members?type=docx&need_notification=false`
  - 请求体：`{ "member_type": "openid", "member_id": open_id, "perm": perm, "type": "user" }`
  - 使用已有的 `get_token()` 获取 tenant_access_token
  - 使用已有的 `send_with_retry()` 处理重试逻辑
  - 解析响应：检查 HTTP status + `code` 字段，非 0 则 bail
  - 特殊处理：code=0 成功；已有权限的重复调用飞书会返回成功，无需额外处理

  **Must NOT do**:
  - 不添加 batch_create 方法
  - 不修改已有方法签名
  - 不引入新依赖

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 单文件单方法新增，模式完全匹配已有的 `create_document`/`batch_update_blocks` 方法
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Task 2)
  - **Blocks**: Task 3
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/docs_sync/client.rs:302-328` — `create_document` 方法：完全相同的 token+retry+response 解析模式，照抄结构
  - `src/docs_sync/client.rs:231-268` — `batch_update_blocks` 方法：rate limit 模式参考（权限 API 无需 rate limit）

  **API/Type References**:
  - `src/docs_sync/client.rs:10` — `FEISHU_BASE_URL` 常量，拼接 URL 用
  - `src/docs_sync/client.rs:48` — `FeishuDocsClient` struct 定义

  **External References**:
  - 飞书官方 Go SDK 确认的 API 签名：`POST /open-apis/drive/v1/permissions/:token/members`
  - Query params: `type=docx`, `need_notification=false`
  - Request body: `BaseMember { member_type, member_id, perm, type }`
  - 来源：`larksuite/oapi-sdk-go` `service/drive/v1/resource.go:1332` 和 `sample/apiall/drivev1/create_permissionMember.go:24`

  **Acceptance Criteria**:
  - [ ] `add_permission_member` 方法存在于 `FeishuDocsClient` impl 块
  - [ ] 方法签名：`pub async fn add_permission_member(&self, document_id: &str, open_id: &str, perm: &str) -> Result<()>`
  - [ ] 使用 `send_with_retry` 包装请求
  - [ ] `cargo clippy --all-targets -- -D warnings` 通过

  **QA Scenarios:**

  ```
  Scenario: 编译通过且无 clippy 警告
    Tool: Bash
    Preconditions: 代码已修改
    Steps:
      1. 运行 `cargo clippy --all-targets -- -D warnings`
      2. 检查退出码为 0
    Expected Result: 零警告零错误
    Failure Indicators: clippy 报错或编译失败
    Evidence: .sisyphus/evidence/task-1-clippy.txt
  ```

  **Commit**: YES
  - Message: `feat(docs-sync): add permission member API to FeishuDocsClient`
  - Files: `src/docs_sync/client.rs`
  - Pre-commit: `cargo clippy --all-targets -- -D warnings`

---

- [x] 2. 共享状态模块 — 暴露 doc_ids + 已分享用户持久化

  **What to do**:
  - 在 `src/docs_sync/mod.rs` 中新增一个轻量的共享状态结构，供 LarkChannel 调用
  - 新增 `pub struct DocsSyncSharer`，持有 `FeishuDocsClient` + lock 文件路径 + 已分享用户文件路径
  - 核心方法：`pub async fn share_all_docs_with(&self, open_id: &str) -> Result<()>`
    - 读取 lock 文件获取所有 doc_id
    - 检查已分享用户文件（JSON: `["ou_xxx"]`），如果 open_id 已存在则跳过
    - 遍历每个 doc_id，调用 `client.add_permission_member(doc_id, open_id, "view")`
    - 全部成功后将 open_id 追加到已分享用户文件
    - 任何单个文档分享失败仅 warn 日志，不中断其余文档
  - 已分享用户文件：`docs_sync_shared_users.json`，与 `docs_sync.lock` 同目录
  - 新增 `pub fn new(app_id, app_secret, lock_path) -> Self` 构造函数

  **Must NOT do**:
  - 不修改 `worker.rs` 的主循环逻辑
  - 不引入新 crate
  - 不添加新 config 字段

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 单文件新增结构体+方法，逻辑简单（读 lock → 调 API → 写 JSON）
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Task 1)
  - **Blocks**: Task 3
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/docs_sync/worker.rs:40-51` — `load_lock`/`save_lock` 函数：JSON 文件读写模式，已分享用户文件照抄此模式
  - `src/docs_sync/worker.rs:32-38` — `lock_file_path` 函数：路径解析模式，shared_users 文件路径同理
  - `src/docs_sync/worker.rs:29` — `LockMap` 类型定义：lock 文件结构 `{ filename: { doc_id, hash } }`

  **API/Type References**:
  - `src/docs_sync/client.rs:40-46` — `FeishuDocsClient` struct：Task 2 需要构造此 client
  - `src/docs_sync/mod.rs:12` — 当前 pub use 导出列表，需要新增 `DocsSyncSharer` 导出

  **Acceptance Criteria**:
  - [ ] `DocsSyncSharer` struct 存在于 `src/docs_sync/mod.rs`
  - [ ] `share_all_docs_with` 方法：读 lock → 检查已分享 → 调 API → 持久化
  - [ ] 已分享用户文件 `docs_sync_shared_users.json` 格式为 `["ou_xxx"]`
  - [ ] 分享失败仅 warn 日志，不 panic
  - [ ] `cargo clippy --all-targets -- -D warnings` 通过

  **QA Scenarios:**
  ```
  Scenario: 编译通过且无 clippy 警告
    Tool: Bash
    Preconditions: 代码已修改
    Steps:
      1. 运行 `cargo clippy --all-targets -- -D warnings`
      2. 检查退出码为 0
    Expected Result: 零警告零错误
    Failure Indicators: clippy 报错或编译失败
    Evidence: .sisyphus/evidence/task-2-clippy.txt
  ```

  **Commit**: YES (groups with Task 1)
  - Message: `feat(docs-sync): add DocsSyncSharer for auto-sharing documents`
  - Files: `src/docs_sync/mod.rs`
  - Pre-commit: `cargo clippy --all-targets -- -D warnings`

---

- [x] 3. LarkChannel 集成 — 检测新用户并触发文档分享

  **What to do**:
  - 在 `src/channels/lark.rs` 的 `LarkChannel` struct 中新增一个可选字段：`docs_sharer: Option<Arc<crate::docs_sync::DocsSyncSharer>>`
  - 新增 `pub fn set_docs_sharer(&mut self, sharer: Arc<DocsSyncSharer>)` 方法
  - 在 `listen_ws()` 的消息处理流程中（提取 `sender_open_id` 之后、构建 `ChannelMessage` 之前），插入分享逻辑：
    - 如果 `self.docs_sharer` 存在，spawn 一个 tokio task 调用 `sharer.share_all_docs_with(sender_open_id)`
    - 使用 `tokio::spawn` 异步执行，不阻塞消息处理
    - 分享结果仅 tracing::warn 记录失败，不影响消息流
  - 同样在 `parse_event_payload()`（webhook 模式）中添加相同逻辑
  - 在 daemon 启动代码中（`src/daemon/mod.rs`），当 docs_sync 和 lark/feishu channel 同时启用时，构造 `DocsSyncSharer` 并注入到 `LarkChannel`
  **Must NOT do**:
  - 不修改 `ChannelMessage` 结构体
  - 不在消息处理主路径上 await 分享结果（必须 spawn）
  - 不引入新 crate
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: 跨模块集成（channels + docs_sync + daemon），需要理解多个文件的交互
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 2 (sequential)
  - **Blocks**: Task 4
  - **Blocked By**: Task 1, Task 2
  **References**:
  **Pattern References**:
  - `src/channels/lark.rs:747` — `sender_open_id` 提取位置，分享逻辑插入点在此之后
  - `src/channels/lark.rs:822-828` — `tokio::spawn` 异步 reaction 模式，分享逻辑照抄此 spawn 模式
  - `src/channels/lark.rs:830-841` — `ChannelMessage` 构建，分享逻辑必须在此之前
  - `src/channels/lark.rs:1229-1232` — webhook 模式的 `open_id` 提取位置
  **API/Type References**:
  - `src/channels/lark.rs:263-287` — `LarkChannel` struct 定义，新增 `docs_sharer` 字段
  - `src/daemon/mod.rs:99` — daemon 启动 docs_sync worker 的位置，需要在此附近构造 sharer 并注入 channel
  - `src/docs_sync/mod.rs` — `DocsSyncSharer` 的 pub use 导出
  **Acceptance Criteria**:
  - [ ] `LarkChannel` struct 新增 `docs_sharer: Option<Arc<DocsSyncSharer>>` 字段
  - [ ] `set_docs_sharer` 方法存在
  - [ ] `listen_ws()` 中 `sender_open_id` 提取后有 spawn 分享逻辑
  - [ ] `parse_event_payload()` 中有相同分享逻辑
  - [ ] daemon 启动代码中构造 sharer 并注入 channel
  - [ ] 分享失败不阻塞消息处理（spawn + warn）
  - [ ] `cargo clippy --all-targets -- -D warnings` 通过
  **QA Scenarios:**
  ```
  Scenario: 全量编译通过且无 clippy 警告
    Tool: Bash
    Preconditions: Task 1, Task 2, Task 3 代码全部完成
    Steps:
      1. 运行 `cargo clippy --all-targets -- -D warnings`
      2. 检查退出码为 0
    Expected Result: 零警告零错误
    Failure Indicators: clippy 报错或编译失败
    Evidence: .sisyphus/evidence/task-3-clippy.txt
  ```
  ```
  Scenario: 分享逻辑使用 spawn 不阻塞消息流
    Tool: Bash (grep)
    Preconditions: 代码已修改
    Steps:
      1. 在 lark.rs 中搜索 `share_all_docs_with`
      2. 确认调用位于 `tokio::spawn` 块内
      3. 确认无 `.await` 直接等待分享结果在消息主路径上
    Expected Result: 所有 share 调用都在 spawn 内
    Failure Indicators: 主路径上直接 await 分享
    Evidence: .sisyphus/evidence/task-3-spawn-check.txt
  ```
  **Commit**: YES
  - Message: `feat(lark): auto-share docs_sync documents on first user message`
  - Files: `src/channels/lark.rs`, `src/daemon/mod.rs`
  - Pre-commit: `cargo clippy --all-targets -- -D warnings`
---
- [x] 4. 全量编译验证 + clippy + test
  **What to do**:
  - 运行 `cargo clippy --all-targets -- -D warnings` 确认零警告
  - 运行 `cargo test` 确认所有测试通过
  - 运行 `cargo build --release` 确认 release 构建成功
  - 检查新增代码无 `unwrap()` 在非测试路径、无 `panic!`、无未使用 import
  **Must NOT do**:
  - 不修改任何源代码（仅验证）
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: 纯验证任务，运行三个命令
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (sequential)
  - **Blocks**: F1-F3
  - **Blocked By**: Task 3
  **References**: None (verification only)
  **Acceptance Criteria**:
  - [ ] `cargo clippy --all-targets -- -D warnings` 退出码 0
  - [ ] `cargo test` 全部通过
  - [ ] `cargo build --release` 成功
  **QA Scenarios:**
  ```
  Scenario: 全量验证通过
    Tool: Bash
    Steps:
      1. 运行 `cargo clippy --all-targets -- -D warnings`，检查退出码 0
      2. 运行 `cargo test`，检查退出码 0
      3. 运行 `cargo build --release`，检查退出码 0
    Expected Result: 三个命令全部成功
    Failure Indicators: 任一命令非零退出
    Evidence: .sisyphus/evidence/task-4-verify.txt
  ```
  **Commit**: NO (verification only)

## Final Verification Wave

- [x] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists. For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [x] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review all changed files for: `unwrap()` in non-test code, empty catches, unused imports. Check AI slop: excessive comments, over-abstraction.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | VERDICT`

- [x] F3. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Wave 1**: `feat(docs-sync): add permission member API to FeishuDocsClient` — `src/docs_sync/client.rs`
- **Wave 2**: `feat(docs-sync): auto-share documents with first Feishu user` — `src/docs_sync/mod.rs`, `src/channels/lark.rs`
- **Wave 3**: No commit (verification only)

---

## Success Criteria

### Verification Commands
```bash
cargo clippy --all-targets -- -D warnings  # Expected: no warnings
cargo test                                   # Expected: all pass
cargo build --release                        # Expected: success
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] New user first message → docs shared (log evidence)
- [ ] Restart → no re-share (log evidence)
