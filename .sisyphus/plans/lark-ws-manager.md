# LarkWsManager — 统一 WS 连接管理器

## TL;DR

> **Quick Summary**: 将 Feishu/Lark WS 连接管理从 `lark.rs` 和 `docs_sync/event_subscriber.rs` 中提取到独立的 `LarkWsManager`，解决同一 app 双 WS 连接导致飞书服务端负载均衡分发事件、消息丢失的 bug。
> 
> **Deliverables**:
> - 新文件 `src/channels/lark_ws_manager.rs`
> - 改造 `src/channels/lark.rs` 订阅 manager
> - 改造 `src/docs_sync/event_subscriber.rs` 订阅 manager
> - 改造 `src/daemon/mod.rs` 创建并传递 manager
> - 改造 `src/channels/mod.rs` 注册模块
> 
> **Estimated Effort**: Medium
> **Parallel Execution**: YES - 3 waves
> **Critical Path**: Task 1 → Task 2/3 (parallel) → Task 4 → Task 5

---

## Context

### Original Request
用户发现连续给飞书机器人发消息时概率性丢消息。根因：`docs_sync` 的 `EventSubscriber` 和 `LarkChannel` 用同一个 `app_id/app_secret` 各自建立独立 WS 长连接，飞书服务端对同一 app 的多个 WS 连接做负载均衡，把事件随机分发到不同连接上。分发到 `docs_sync` 连接的 `im.message.receive_v1` 消息被 ACK 后静默丢弃。

### Interview Summary
**Key Discussions**:
- 用户要求新建 `LarkWsManager` 拆分 WS 连接管理逻辑
- `lark.rs` 和 `docs_sync` 订阅 manager，manager 向两者分发消息
- 便于之后扩展

**Research Findings**:
- 飞书官方 Go SDK 用 `go handleMessage()` 每条消息独立 goroutine
- PbFrame codec 在 `lark.rs` 和 `event_subscriber.rs` 中完全重复
- `gateway/sse.rs` 已有 `broadcast::channel(256)` 模式可参考
- `vpn/health.rs` 已有 `CancellationToken` 生命周期模式可参考

### Metis Review
**Identified Gaps** (addressed):
- Manager 需要支持 LarkPlatform（Lark 国际版 vs Feishu 中国版）
- Webhook 模式下 LarkChannel 不用 WS，但 docs_sync 可能仍需要
- broadcast channel lag 需要处理 `RecvError::Lagged`
- 启动顺序：manager 必须在订阅者之前创建
- Feature gate：manager 需要在 `channel-lark` 或 `feishu-docs-sync` 任一启用时可用

---

## Work Objectives

### Core Objective
将 WS 连接管理提取为独立的 `LarkWsManager`，确保同一 app 只有一个 WS 连接，通过 broadcast 向所有订阅者分发事件。

### Concrete Deliverables
- `src/channels/lark_ws_manager.rs` — 新文件
- `src/channels/lark.rs` — 改造 `listen_ws()` 订阅 manager
- `src/docs_sync/event_subscriber.rs` — 改造为订阅 manager
- `src/daemon/mod.rs` — 创建 manager 并传递
- `src/channels/mod.rs` — 注册新模块

### Definition of Done
- [ ] `cargo build --features channel-lark,feishu-docs-sync` 编译通过
- [ ] `cargo build --features channel-lark` 独立编译通过
- [ ] `cargo build --features feishu-docs-sync` 独立编译通过
- [ ] `cargo test` 所有现有测试通过
- [ ] `grep -rn "struct PbFrame" src/` 只在 `lark_ws_manager.rs` 中有一处定义
- [ ] `grep -rn "connect_async" src/channels/lark.rs src/docs_sync/event_subscriber.rs` 返回零匹配

### Must Have
- 单一 WS 连接 per app
- broadcast 分发机制
- 心跳、ACK、帧解析、fragment reassembly 在 manager 中
- lark.rs 保留所有业务逻辑（dedup、allowlist、content parsing、reactions）
- event_subscriber.rs 保留 document_id 过滤和 drive.file.edit_v1 过滤
- 支持 LarkPlatform（Lark/Feishu）

### Must NOT Have (Guardrails)
- 不改 Channel trait 或 `listen()` 签名
- 不动 `send()`、`send_draft()`、`health_check()` 等 HTTP API 方法
- 不动 `worker.rs` 的 sync 逻辑
- 不加新 trait 或抽象层（只有 `LarkWsManager` struct）
- 不加新依赖到 `Cargo.toml`
- 不动 `DocsSyncSharer` 集成
- 不改 ACK 时序（保持立即 ACK）
- 不创建 "shared types" crate

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
- **Infrastructure exists**: YES
- **Automated tests**: Tests-after (现有测试必须通过，新增 manager 单元测试)
- **Framework**: cargo test

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Library/Module**: Use Bash (cargo) — Build, test, grep verification
- **Integration**: Use Bash — cargo build with various feature combinations

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation):
├── Task 1: 创建 lark_ws_manager.rs + 注册模块 [deep]

Wave 2 (Consumers — MAX PARALLEL):
├── Task 2: 改造 lark.rs 订阅 manager [deep]
├── Task 3: 改造 docs_sync event_subscriber.rs 订阅 manager [unspecified-high]

Wave 3 (Integration):
├── Task 4: 改造 daemon/mod.rs 创建并传递 manager [unspecified-high]
├── Task 5: 编译验证 + 测试 + 清理 [deep]

Wave FINAL (Review):
├── Task F1: Plan compliance audit [oracle]
├── Task F2: Code quality review [unspecified-high]
├── Task F3: Scope fidelity check [deep]

Critical Path: Task 1 → Task 2/3 → Task 4 → Task 5 → F1-F3
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | — | 2, 3, 4 |
| 2 | 1 | 5 |
| 3 | 1 | 5 |
| 4 | 1 | 5 |
| 5 | 2, 3, 4 | F1-F3 |

### Agent Dispatch Summary

- **Wave 1**: 1 task — T1 → `deep`
- **Wave 2**: 2 tasks — T2 → `deep`, T3 → `unspecified-high`
- **Wave 3**: 2 tasks — T4 → `unspecified-high`, T5 → `deep`
- **FINAL**: 3 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `deep`

---

## TODOs

- [ ] 1. 创建 `src/channels/lark_ws_manager.rs` + 注册模块

  **What to do**:
  - 新建 `src/channels/lark_ws_manager.rs`
  - 从 `lark.rs:34-93` 提取 `PbHeader`、`PbFrame`（含 `header_value`）、`WsClientConfig`、`WsEndpointResp`、`WsEndpoint`，改为 `pub(crate)`
  - 从 `lark.rs:558-582` 提取 `get_ws_endpoint()` 逻辑作为 manager 方法，接受 `LarkPlatform` 参数决定 ws_base_url 和 locale_header
  - 从 `lark.rs:587-754` 提取 WS 连接循环核心：连接建立、initial ping、heartbeat tick、timeout check、帧读取、CONTROL 帧处理（pong + ping_interval 校准）、ACK 发送、fragment reassembly
  - 定义 broadcast 事件类型：`pub struct LarkWsEvent { pub event_type: String, pub payload: Vec<u8> }`
  - 解码 DATA 帧后，解析 event envelope 提取 `event_type`，通过 `broadcast::Sender<LarkWsEvent>` 分发
  - 提供 `pub fn new(app_id, app_secret, is_feishu, capacity) -> Self`
  - 提供 `pub fn subscribe(&self) -> broadcast::Receiver<LarkWsEvent>`
  - 提供 `pub async fn run(&self) -> anyhow::Result<()>` 内部自动重连
  - 使用 `broadcast::channel(256)` 匹配 `gateway/sse.rs` 模式
  - 在 `src/channels/mod.rs` 中添加 `pub mod lark_ws_manager;`，feature gate: `#[cfg(any(feature = "channel-lark", feature = "feishu-docs-sync"))]`
  - 修复 fragment reassembly 中 `seq_num >= sum` 的处理：当单帧处理（与上游一致），不丢弃

  **Must NOT do**:
  - 不在 manager 中包含任何业务逻辑（dedup、allowlist、content parsing、reactions）
  - 不加新依赖到 Cargo.toml
  - 不改 Channel trait

  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 核心架构模块，需要精确提取和重组大量代码
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 (alone)
  - **Blocks**: Tasks 2, 3, 4
  - **Blocked By**: None

  **References**:

  **Pattern References**:
  - `src/channels/lark.rs:34-68` — PbHeader/PbFrame 定义（提取到 manager）
  - `src/channels/lark.rs:71-93` — WsClientConfig/WsEndpointResp/WsEndpoint（提取到 manager）
  - `src/channels/lark.rs:558-582` — get_ws_endpoint() 方法（提取到 manager）
  - `src/channels/lark.rs:587-754` — listen_ws() 中 WS 连接循环核心（提取到 manager）
  - `src/channels/lark.rs:155-168` — should_refresh_last_recv() 函数（提取到 manager）
  - `src/gateway/sse.rs:413` — broadcast::channel(256) 模式参考

  **API/Type References**:
  - `src/docs_sync/event_subscriber.rs:24-79` — 重复的 PbFrame/WsClientConfig 定义（验证一致性）
  - `src/docs_sync/event_subscriber.rs:82-94` — DriveEvent/DriveEventHeader（docs_sync 自己保留）

  **External References**:
  - 飞书官方 Go SDK `ws/client.go` — `receiveMessageLoop` + `handleMessage` 模式参考

  **WHY Each Reference Matters**:
  - `lark.rs:34-93`: 这些类型是 WS 帧编解码的核心，必须精确提取
  - `lark.rs:558-582`: WS endpoint 获取逻辑，manager 需要复用
  - `lark.rs:587-754`: 整个 WS 连接循环的核心，是提取的主体
  - `gateway/sse.rs:413`: broadcast channel 的 codebase 先例，确保模式一致

  **Acceptance Criteria**:

  - [ ] `src/channels/lark_ws_manager.rs` 存在且包含 `LarkWsManager` struct
  - [ ] `src/channels/mod.rs` 包含 `pub mod lark_ws_manager;`
  - [ ] `cargo build --features channel-lark` 编译通过（即使 lark.rs 和 event_subscriber.rs 尚未改造）

  **QA Scenarios:**

  ```
  Scenario: Manager 模块编译通过
    Tool: Bash (cargo)
    Preconditions: Task 1 代码已写入
    Steps:
      1. cargo build --features channel-lark 2>&1
      2. 检查退出码为 0
    Expected Result: 编译成功，无 error
    Evidence: .sisyphus/evidence/task-1-compile.txt

  Scenario: PbFrame 只在 manager 中定义
    Tool: Bash (grep)
    Preconditions: Task 1 完成
    Steps:
      1. grep -rn "struct PbFrame" src/channels/lark_ws_manager.rs
      2. 确认有且仅有一处定义
    Expected Result: 1 match in lark_ws_manager.rs
    Evidence: .sisyphus/evidence/task-1-pbframe-unique.txt
  ```

  **Commit**: YES
  - Message: `refactor(channels): add LarkWsManager for shared WS connection`
  - Files: `src/channels/lark_ws_manager.rs`, `src/channels/mod.rs`
  - Pre-commit: `cargo build --features channel-lark`


- [ ] 2. 改造 `src/channels/lark.rs` 订阅 manager
  **What to do**:
  - 删除 `lark.rs` 中的 `PbHeader`、`PbFrame`、`WsClientConfig`、`WsEndpointResp`、`WsEndpoint` 定义（34-93行），改为 `use super::lark_ws_manager::{...}`
  - 删除 `get_ws_endpoint()` 方法（558-582行）
  - 删除 `should_refresh_last_recv()` 函数（155-168行）
  - 给 `LarkChannel` 添加 `ws_manager: Option<Arc<LarkWsManager>>` 字段
  - 重写 `listen_ws()`：不再自己建 WS 连接，改为调用 `self.ws_manager.subscribe()` 获取 `broadcast::Receiver<LarkWsEvent>`，在循环中 `recv()` 事件
  - 保留所有业务逻辑：dedup（ws_seen_ids）、allowlist、content parsing（text/post/image/file/audio/media）、ACK reaction、overflow buffer、group chat @-mention gating、auto-share docs
  - 处理 `RecvError::Lagged(n)` — log warn 并继续
  - 只处理 `event_type == "im.message.receive_v1"` 的事件
  **Must NOT do**:
  - 不动 `send()`、`send_draft()`、`update_draft()`、`finalize_draft()`、`cancel_draft()`、`start_typing()`、`stop_typing()`、`health_check()` 等方法
  - 不动 webhook 模式（`listen_http`）
  - 不动 `DocsSyncSharer` 集成
  - 不改 Channel trait 签名
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 核心 channel 改造，需要精确保留所有业务逻辑
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 3)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 5
  - **Blocked By**: Task 1
  **References**:
  - `src/channels/lark.rs:34-93` — 要删除的类型定义
  - `src/channels/lark.rs:558-582` — 要删除的 get_ws_endpoint()
  - `src/channels/lark.rs:587-897` — 要重写的 listen_ws() 完整函数
  - `src/channels/lark.rs:155-168` — 要删除的 should_refresh_last_recv()
  - `src/channels/lark.rs:263-289` — LarkChannel struct 定义（添加 ws_manager 字段）
  - `src/channels/lark.rs:794-804` — dedup 逻辑（保留）
  - `src/channels/lark.rs:807-847` — content parsing 逻辑（保留）
  **Acceptance Criteria**:
  - [ ] `grep -rn "struct PbFrame" src/channels/lark.rs` 返回零匹配
  - [ ] `grep -rn "connect_async" src/channels/lark.rs` 返回零匹配
  - [ ] `grep -rn "get_ws_endpoint" src/channels/lark.rs` 返回零匹配
  - [ ] `cargo build --features channel-lark` 编译通过
  **QA Scenarios:**
  ```
  Scenario: lark.rs 不再包含 WS 连接代码
    Tool: Bash (grep)
    Steps:
      1. grep -rn "connect_async" src/channels/lark.rs
      2. grep -rn "struct PbFrame" src/channels/lark.rs
      3. 两者都应返回零匹配
    Expected Result: 0 matches each
    Evidence: .sisyphus/evidence/task-2-no-ws-code.txt
  Scenario: lark.rs 编译通过
    Tool: Bash (cargo)
    Steps:
      1. cargo build --features channel-lark 2>&1
    Expected Result: 编译成功
    Evidence: .sisyphus/evidence/task-2-compile.txt
  ```
  **Commit**: YES (groups with Task 3, 4)
  - Message: `refactor(channels,docs_sync): subscribe to LarkWsManager instead of own WS`
  - Files: `src/channels/lark.rs`

- [ ] 3. 改造 `src/docs_sync/event_subscriber.rs` 订阅 manager
  **What to do**:
  - 删除 `event_subscriber.rs` 中的 `PbHeader`、`PbFrame`、`WsClientConfig`、`WsEndpointResp`、`WsEndpoint` 定义（24-79行）
  - 删除 `get_ws_endpoint()` 方法（121-145行）
  - 删除整个 WS 连接循环（`subscribe_ws` 方法，约 160-348行）
  - 给 `EventSubscriber` 添加 `ws_manager: Arc<LarkWsManager>` 字段，修改 `new()` 接受 manager
  - 重写 `run()` 方法：调用 `self.ws_manager.subscribe()` 获取 receiver，在循环中 `recv()` 事件
  - 只处理 `event_type == "drive.file.edit_v1"` 的事件
  - 保留 document_id 过滤逻辑（329-340行）
  - 保留 `DriveEvent`/`DriveEventHeader` 类型定义（82-94行）
  - 处理 `RecvError::Lagged(n)` — log warn 并继续（docs_sync 丢几条无所谓，会 pull latest）
  **Must NOT do**:
  - 不动 `worker.rs` 的 sync 逻辑
  - 不加新依赖
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: 较简单的订阅改造，主要是删除重复代码 + 订阅 broadcast
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (with Task 2)
  - **Parallel Group**: Wave 2
  - **Blocks**: Task 4, 5
  - **Blocked By**: Task 1
  **References**:
  - `src/docs_sync/event_subscriber.rs:24-79` — 要删除的重复类型定义
  - `src/docs_sync/event_subscriber.rs:82-94` — DriveEvent/DriveEventHeader（保留）
  - `src/docs_sync/event_subscriber.rs:100-118` — EventSubscriber struct + new()（修改）
  - `src/docs_sync/event_subscriber.rs:160-348` — subscribe_ws() 完整方法（删除重写）
  - `src/docs_sync/event_subscriber.rs:320-326` — drive.file.edit_v1 过滤（保留）
  - `src/docs_sync/event_subscriber.rs:329-340` — document_id 过滤（保留）
  - `src/docs_sync/worker.rs:169-173` — EventSubscriber 创建和启动点（需要修改传参）
  **Acceptance Criteria**:
  - [ ] `grep -rn "struct PbFrame" src/docs_sync/` 返回零匹配
  - [ ] `grep -rn "connect_async" src/docs_sync/` 返回零匹配
  - [ ] `cargo build --features feishu-docs-sync` 编译通过
  **QA Scenarios:**
  ```
  Scenario: event_subscriber.rs 不再包含 WS 连接代码
    Tool: Bash (grep)
    Steps:
      1. grep -rn "connect_async" src/docs_sync/event_subscriber.rs
      2. grep -rn "struct PbFrame" src/docs_sync/
    Expected Result: 0 matches each
    Evidence: .sisyphus/evidence/task-3-no-ws-code.txt
  ```
  **Commit**: YES (groups with Task 2, 4)
  - Message: `refactor(channels,docs_sync): subscribe to LarkWsManager instead of own WS`
  - Files: `src/docs_sync/event_subscriber.rs`, `src/docs_sync/worker.rs`
- [ ] 4. 改造 `src/daemon/mod.rs` + `src/docs_sync/worker.rs` 创建并传递 manager
  **What to do**:
  - 在 `daemon/mod.rs` 中，当 `channel-lark` 或 `feishu-docs-sync` feature 启用时，创建 `Arc<LarkWsManager>`
  - 从 config 中解析 app_id/app_secret 和 platform（feishu vs lark），传给 manager 构造函数
  - `tokio::spawn(manager.clone().run())` 启动 WS 连接循环
  - 将 `Arc<LarkWsManager>` 传递给 `start_channels()` 和 `run_worker()`
  - 修改 `start_channels()` 签名接受 `Option<Arc<LarkWsManager>>`，传递给 LarkChannel 构造
  - 修改 `docs_sync::run_worker()` 签名接受 `Option<Arc<LarkWsManager>>`，传递给 EventSubscriber
  - manager 必须在订阅者之前启动
  **Must NOT do**:
  - 不改变现有的 component supervisor 架构
  - 不加新 config key
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: 集成布线，需要理解 daemon 启动流程
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3
  - **Blocks**: Task 5
  - **Blocked By**: Tasks 1, 2, 3
  **References**:
  - `src/daemon/mod.rs:43-58` — start_channels() 调用点
  - `src/daemon/mod.rs:89-106` — docs_sync 启动点
  - `src/docs_sync/worker.rs:97-174` — run() 函数，EventSubscriber 创建点在 169-173行
  - `src/channels/mod.rs:3340-3360` — spawn_supervised_listener 和 channel 创建逻辑
  **Acceptance Criteria**:
  - [ ] `cargo build --features channel-lark,feishu-docs-sync` 编译通过
  **QA Scenarios:**
  ```
  Scenario: 完整 feature 组合编译
    Tool: Bash (cargo)
    Steps:
      1. cargo build --features channel-lark,feishu-docs-sync 2>&1
    Expected Result: 编译成功
    Evidence: .sisyphus/evidence/task-4-compile-full.txt
  ```
  **Commit**: YES (groups with Task 2, 3)
  - Message: `refactor(channels,docs_sync): subscribe to LarkWsManager instead of own WS`
  - Files: `src/daemon/mod.rs`, `src/docs_sync/worker.rs`, `src/channels/mod.rs`
- [ ] 5. 编译验证 + 测试 + 清理
  **What to do**:
  - 运行 `cargo fmt --all -- --check` 修复格式
  - 运行 `cargo clippy --all-targets -- -D warnings` 修复 lint
  - 运行 `cargo test` 确保所有现有测试通过
  - 验证 feature 独立编译：`cargo build --features channel-lark`、`cargo build --features feishu-docs-sync`
  - 验证 PbFrame 唯一性：`grep -rn "struct PbFrame" src/` 只在 lark_ws_manager.rs
  - 验证无残留 WS 连接：`grep -rn "connect_async" src/channels/lark.rs src/docs_sync/`
  - 清理 upstream_lark.rs 等临时文件（如果存在）
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: 全面验证，需要运行多个命令并修复问题
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (after Task 4)
  - **Blocks**: F1-F3
  - **Blocked By**: Tasks 2, 3, 4
  **Acceptance Criteria**:
  - [ ] `cargo fmt --all -- --check` 无 diff
  - [ ] `cargo clippy --all-targets -- -D warnings` 无 error
  - [ ] `cargo test` 全部通过
  - [ ] 三种 feature 组合均编译通过
  **QA Scenarios:**
  ```
  Scenario: 完整验证套件
    Tool: Bash
    Steps:
      1. cargo fmt --all -- --check
      2. cargo clippy --all-targets -- -D warnings
      3. cargo test
      4. cargo build --features channel-lark
      5. cargo build --features feishu-docs-sync
      6. cargo build --features channel-lark,feishu-docs-sync
      7. grep -rn "struct PbFrame" src/ | wc -l  # expect 1
      8. grep -rn "connect_async" src/channels/lark.rs src/docs_sync/ | wc -l  # expect 0
    Expected Result: 全部通过
    Evidence: .sisyphus/evidence/task-5-full-verify.txt
  ```
  **Commit**: YES
  - Message: `chore: cleanup duplicated codec and verify feature gates`
  - Pre-commit: `cargo test`
---

## Final Verification Wave

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists. For each "Must NOT Have": search codebase for forbidden patterns. Check evidence files exist.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | VERDICT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review all changed files for: `as any`/`@ts-ignore` equivalents, empty catches, unused imports. Check AI slop: excessive comments, over-abstraction.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | VERDICT`

- [ ] F3. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Task 1**: `refactor(channels): add LarkWsManager for shared WS connection` — `src/channels/lark_ws_manager.rs`, `src/channels/mod.rs`
- **Task 2+3+4**: `refactor(channels,docs_sync): subscribe to LarkWsManager instead of own WS` — `src/channels/lark.rs`, `src/docs_sync/event_subscriber.rs`, `src/daemon/mod.rs`
- **Task 5**: `chore: cleanup duplicated PbFrame codec and verify feature gates` — any remaining cleanup

---

## Success Criteria

### Verification Commands
```bash
cargo build --features channel-lark,feishu-docs-sync  # Expected: success
cargo build --features channel-lark                     # Expected: success
cargo build --features feishu-docs-sync                 # Expected: success
cargo test                                              # Expected: all pass
cargo fmt --all -- --check                              # Expected: no diff
cargo clippy --all-targets -- -D warnings               # Expected: no errors
grep -rn "struct PbFrame" src/                          # Expected: 1 match in lark_ws_manager.rs
grep -rn "connect_async" src/channels/lark.rs src/docs_sync/event_subscriber.rs  # Expected: 0 matches
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass
- [ ] Single WS connection per app confirmed
- [ ] broadcast 分发机制工作正常
