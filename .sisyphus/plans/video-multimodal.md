# Video Multimodal Support for ZeroClaw

## TL;DR

> **Quick Summary**: Extend ZeroClaw's multimodal pipeline from image-only to image+video, enabling custom Qwen 3.5 providers to understand video content via `[VIDEO:...]` markers. Video uses URL pass-through (not base64 normalization like images) due to size constraints.
> 
> **Deliverables**:
> - `[VIDEO:url]` marker parsing, validation, and provider dispatch in `src/multimodal.rs`
> - `VideoUrl` content part in `compatible.rs` and `openrouter.rs` providers
> - `video: bool` capability flag in `ProviderCapabilities`
> - Video config fields in `MultimodalConfig` (`max_videos`, `max_video_size_mb`)
> - Telegram incoming video → `[VIDEO:...]` marker generation
> - WhatsApp incoming video → `[VIDEO:...]` marker generation
> - Full TDD test coverage for all changes
> 
> **Estimated Effort**: Medium-Large
> **Parallel Execution**: YES — 4 waves
> **Critical Path**: Task 1 → Task 3 → Task 5 → Task 8 → Task 10 → Task 13 → Final

---

## Context

### Original Request
用户使用 custom 源的 Qwen 3.5（原生支持多模态视频），希望 ZeroClaw 能让模型读懂网上的视频。当前 ZeroClaw 仅支持图片多模态。

### Interview Summary
**Key Discussions**:
- ZeroClaw 当前仅支持 `[IMAGE:...]` 标记的图片多模态
- 用户的 custom Qwen 3.5 provider 原生支持视频输入
- Custom provider 走 `compatible.rs`（OpenAI 兼容格式）
- DashScope/Qwen 使用 `video_url` content part type
- 用户要求 TDD 开发，在 WSL2 中测试

**Research Findings**:
- DashScope 视频格式: `{"type": "video_url", "video_url": {"url": "..."}}`，支持 URL、base64、本地文件
- OpenAI 官方 API 无 `video_url`；Anthropic 无视频支持；Gemini 用 `inline_data`
- 视频不能走 base64 归一化管道（文件太大），必须 URL 直传
- Ollama 视频支持不明确，本次不涉及

### Metis Review
**Identified Gaps** (addressed):
- 视频不能复用图片的 base64 归一化管道 → 视频走 URL pass-through，不做 base64 转换
- 需要 `video` capability flag 防止视频 marker 泄漏到不支持的 provider → 添加 `video: bool` 到 `ProviderCapabilities`
- 用户的 Qwen 端点是 `custom:https://...` 格式 → 改动集中在 `compatible.rs`
- 需要明确 scope boundary：本次不改 Ollama、Gemini、Anthropic provider

---

## Work Objectives

### Core Objective
让 ZeroClaw 的 custom provider（compatible.rs）能够将 `[VIDEO:url]` 标记转换为 DashScope 兼容的 `video_url` content part，发送给 Qwen 3.5 模型理解视频内容。

### Concrete Deliverables
- `src/multimodal.rs`: 视频 marker 解析、计数、验证、URL pass-through
- `src/config/schema.rs`: `MultimodalConfig` 视频字段
- `src/providers/traits.rs`: `video` capability
- `src/providers/compatible.rs`: `VideoUrl` MessagePart
- `src/providers/openrouter.rs`: `VideoUrl` MessagePart
- `src/channels/telegram.rs`: 接收视频 → 生成 `[VIDEO:...]` marker
- `src/channels/whatsapp.rs`: 接收视频 → 生成 `[VIDEO:...]` marker

### Definition of Done
- [ ] `cargo test` 全部通过（WSL2 环境）
- [ ] `cargo clippy --all-targets -- -D warnings` 无警告
- [ ] `cargo fmt --all -- --check` 格式正确
- [ ] 所有新增功能有对应的单元测试（TDD）
- [ ] `[VIDEO:https://example.com/test.mp4]` marker 能被正确解析并转换为 `video_url` content part

### Must Have
- `[VIDEO:url]` marker 解析和验证
- 视频 URL pass-through（不做 base64 转换）
- `video: bool` provider capability flag
- `VideoUrl` content part in compatible.rs
- 视频 MIME 类型白名单
- 视频大小/数量限制配置
- Telegram 接收视频生成 marker
- TDD：每个功能先写测试

### Must NOT Have (Guardrails)
- 不做视频 base64 编码/归一化（视频文件太大，走 URL 直传）
- 不改 Ollama provider（视频支持不明确）
- 不改 Anthropic/Gemini provider（它们有自己的视频格式）
- 不做视频抽帧（frame extraction）功能
- 不做视频转码/压缩
- 不添加 `fps`、`min_pixels`、`max_pixels` 等 Qwen 特有参数（保持 OpenAI 兼容格式简洁）
- 不做音频多模态（不在本次 scope 内）
- 不引入新的外部依赖

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES（`cargo test` 已有完整测试基础设施）
- **Automated tests**: TDD（RED → GREEN → REFACTOR）
- **Framework**: `cargo test`（Rust 内置 + `#[tokio::test]` for async）
- **TDD**: 每个 task 先写失败测试，再实现使其通过

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Unit tests**: `cargo test` — 运行特定模块测试
- **Lint/Format**: `cargo clippy` + `cargo fmt --check`
- **Integration**: `cargo test --test` 集成测试（如有）

### WSL2 Testing Environment
所有 `cargo test`、`cargo clippy`、`cargo fmt` 命令在 WSL2 中执行。

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — config + traits + multimodal core):
├── Task 1: MultimodalConfig 视频字段 [quick]
├── Task 2: ProviderCapabilities video flag [quick]
├── Task 3: multimodal.rs 视频 marker 解析 [deep]
├── Task 4: multimodal.rs 视频 MIME 类型 [quick]

Wave 2 (Core pipeline — multimodal preparation + provider dispatch):
├── Task 5: multimodal.rs prepare_messages 视频处理 [deep]
├── Task 6: multimodal.rs PreparedMessages 扩展 [quick]
├── Task 7: compatible.rs VideoUrl MessagePart [deep]
├── Task 8: openrouter.rs VideoUrl MessagePart [unspecified-high]

Wave 3 (Channel integration — Telegram + WhatsApp):
├── Task 9: telegram.rs parse_attachment_metadata 视频支持 [unspecified-high]
├── Task 10: telegram.rs format_attachment_content 视频 marker [unspecified-high]
├── Task 11: whatsapp.rs 视频消息解析 [unspecified-high]

Wave 4 (Integration + agent loop):
├── Task 12: agent/loop_.rs 视频 capability 检查 [quick]
├── Task 13: 集成测试 — 端到端视频 marker 流 [deep]

Wave FINAL (Verification):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real QA — cargo test + clippy + fmt (unspecified-high)
├── Task F4: Scope fidelity check (deep)

Critical Path: Task 1 → Task 3 → Task 5 → Task 7 → Task 12 → Task 13 → F1-F4
Parallel Speedup: ~60% faster than sequential
Max Concurrent: 4 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks | Wave |
|------|-----------|--------|------|
| 1 | — | 5, 6 | 1 |
| 2 | — | 7, 8, 12 | 1 |
| 3 | — | 5, 7, 8 | 1 |
| 4 | — | 5 | 1 |
| 5 | 1, 3, 4 | 13 | 2 |
| 6 | 1 | 12, 13 | 2 |
| 7 | 2, 3 | 13 | 2 |
| 8 | 2, 3 | 13 | 2 |
| 9 | — | 10 | 3 |
| 10 | 3, 9 | 13 | 3 |
| 11 | 3 | 13 | 3 |
| 12 | 2, 6 | 13 | 4 |
| 13 | 5, 7, 10, 12 | F1-F4 | 4 |

### Agent Dispatch Summary

- **Wave 1**: 4 tasks — T1 `quick`, T2 `quick`, T3 `deep`, T4 `quick`
- **Wave 2**: 4 tasks — T5 `deep`, T6 `quick`, T7 `deep`, T8 `unspecified-high`
- **Wave 3**: 3 tasks — T9 `unspecified-high`, T10 `unspecified-high`, T11 `unspecified-high`
- **Wave 4**: 2 tasks — T12 `quick`, T13 `deep`
- **FINAL**: 4 tasks — F1 `oracle`, F2 `unspecified-high`, F3 `unspecified-high`, F4 `deep`

---

## TODOs


### Wave 1 — Foundation

- [ ] 1. MultimodalConfig 视频字段 (TDD)

  **What to do**:
  - RED: 写测试验证 `MultimodalConfig` 有 `max_videos: usize` 和 `max_video_size_mb: usize` 字段
  - RED: 写测试验证 `effective_video_limits()` 方法返回 clamped 值（max_videos: 1..8, max_video_size_mb: 1..100）
  - RED: 写测试验证 `Default` impl 设置 `max_videos = 2`, `max_video_size_mb = 20`
  - GREEN: 在 `MultimodalConfig` struct 添加字段，添加 serde default 函数，实现 `effective_video_limits()`
  - REFACTOR: 确保与现有 `effective_limits()` 风格一致

  **Must NOT do**:
  - 不修改现有 `max_images` / `max_image_size_mb` 字段的行为
  - 不添加 `fps`、`min_pixels`、`max_pixels` 等 Qwen 特有参数

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4)
  - **Blocks**: Tasks 5, 6
  - **Blocked By**: None

  **References**:
  - `src/config/schema.rs:502-541` — 现有 `MultimodalConfig` struct、default 函数、`effective_limits()` 方法。新字段必须完全复制这个模式
  - `src/config/schema.rs:3534` — `AppConfig` Default impl 中 `multimodal: MultimodalConfig::default()`，确认新字段有 default

  **Acceptance Criteria**:
  - [ ] `cargo test config::` 通过，包含新增的视频字段测试
  - [ ] `MultimodalConfig { max_videos: 2, max_video_size_mb: 20, .. }` 是 default
  - [ ] `effective_video_limits()` clamp 到 (1..8, 1..100)

  **QA Scenarios**:
  ```
  Scenario: Default video limits
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw config::tests -- --nocapture 2>&1
      2. 检查输出包含 test result: ok
    Expected Result: 所有 config 测试通过
    Evidence: .sisyphus/evidence/task-1-config-defaults.txt

  Scenario: Serde deserialization without video fields
    Tool: Bash (cargo test)
    Steps:
      1. 写测试：反序列化不含 max_videos 的 TOML → 验证使用 default 值
      2. cargo test multimodal_config_defaults
    Expected Result: 缺失字段使用 default (max_videos=2, max_video_size_mb=20)
    Evidence: .sisyphus/evidence/task-1-serde-defaults.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `feat(config): add video limits to MultimodalConfig`
  - Files: `src/config/schema.rs`
  - Pre-commit: `cargo test config`

- [ ] 2. ProviderCapabilities video flag (TDD)

  **What to do**:
  - RED: 写测试验证 `ProviderCapabilities` 有 `video: bool` 字段，default 为 `false`
  - RED: 写测试验证 `Provider::supports_video()` 方法返回 `self.capabilities().video`
  - GREEN: 在 `ProviderCapabilities` struct 添加 `pub video: bool`
  - GREEN: 在 `Provider` trait 添加 `fn supports_video(&self) -> bool` 默认实现
  - REFACTOR: 确保与 `supports_vision()` 风格完全一致

  **Must NOT do**:
  - 不修改任何具体 provider 的 `capabilities()` 返回值（后续 task 处理）
  - 不改变 `vision` 字段的行为

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 3, 4)
  - **Blocks**: Tasks 7, 8, 12
  - **Blocked By**: None

  **References**:
  - `src/providers/traits.rs:225-236` — `ProviderCapabilities` struct，当前只有 `native_tool_calling` 和 `vision`
  - `src/providers/traits.rs:386-389` — `supports_vision()` 方法，`supports_video()` 必须完全复制此模式
  - `src/providers/traits.rs:225` — `#[derive(Debug, Clone, Default, PartialEq, Eq)]`，Default derive 会自动给 `video` 设为 `false`

  **Acceptance Criteria**:
  - [ ] `ProviderCapabilities::default().video == false`
  - [ ] `supports_video()` 返回 `self.capabilities().video`
  - [ ] `cargo test providers::` 通过

  **QA Scenarios**:
  ```
  Scenario: Default video capability is false
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw providers -- video --nocapture 2>&1
      2. 检查输出包含 test result: ok
    Expected Result: video capability 默认 false
    Evidence: .sisyphus/evidence/task-2-capability-default.txt
  ```

  **Commit**: YES (groups with Wave 1)
  - Message: `feat(providers): add video capability flag to ProviderCapabilities`
  - Files: `src/providers/traits.rs`
  - Pre-commit: `cargo test providers`
- [ ] 3. multimodal.rs 视频 marker 解析 (TDD)
  **What to do**:
  - RED: 写测试验证 `parse_video_markers(content)` 能从文本中提取 `[VIDEO:url]` 标记
  - RED: 写测试验证 `count_video_markers(messages)` 正确计数用户消息中的视频标记
  - RED: 写测试验证 `contains_video_markers(messages)` 返回 bool
  - RED: 写测试验证空 marker `[VIDEO:]` 被保留不解析（与 IMAGE 行为一致）
  - GREEN: 添加 `VIDEO_MARKER_PREFIX = "[VIDEO:"`
  - GREEN: 实现 `parse_video_markers`（复制 `parse_image_markers` 模式，替换前缀）
  - GREEN: 实现 `count_video_markers` 和 `contains_video_markers`
  - REFACTOR: 考虑是否提取通用 `parse_markers(prefix, content)` 函数减少重复
  **Must NOT do**:
  - 不修改 `parse_image_markers` 的行为
  - 不做视频内容下载或验证（后续 task 处理）
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4)
  - **Blocks**: Tasks 5, 7, 8, 10, 11
  - **Blocked By**: None
  **References**:
  - `src/multimodal.rs:7` — `IMAGE_MARKER_PREFIX` 常量定义，VIDEO 版本必须完全复制此模式
  - `src/multimodal.rs:53-86` — `parse_image_markers` 函数，这是 `parse_video_markers` 的模板
  - `src/multimodal.rs:88-98` — `count_image_markers` 和 `contains_image_markers`，视频版本复制此模式
  - `src/multimodal.rs:451-468` — 现有 `parse_image_markers` 测试，视频测试必须覆盖相同场景
  **Acceptance Criteria**:
  - [ ] `parse_video_markers("Check [VIDEO:https://example.com/v.mp4]")` 返回 cleaned text + refs
  - [ ] `count_video_markers` 只计数 user role 消息
  - [ ] `parse_video_markers("[VIDEO:]")` 保留原文（空 marker 不解析）
  - [ ] 混合 IMAGE 和 VIDEO marker 互不干扰
  **QA Scenarios**:
  ```
  Scenario: Parse video markers from mixed content
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw multimodal::tests::parse_video -- --nocapture 2>&1
      2. 检查输出包含 test result: ok
    Expected Result: 所有 video marker 解析测试通过
    Evidence: .sisyphus/evidence/task-3-parse-video-markers.txt
  Scenario: Video and image markers coexist
    Tool: Bash (cargo test)
    Steps:
      1. 写测试：内容同时包含 [IMAGE:a.png] 和 [VIDEO:v.mp4]
      2. 验证 parse_image_markers 只提取 image，parse_video_markers 只提取 video
    Expected Result: 两个解析器互不干扰
    Evidence: .sisyphus/evidence/task-3-mixed-markers.txt
  ```
  **Commit**: YES (groups with Wave 1)
  - Message: `feat(multimodal): add video marker parsing`
  - Files: `src/multimodal.rs`
  - Pre-commit: `cargo test multimodal`
- [ ] 4. multimodal.rs 视频 MIME 类型 (TDD)
  **What to do**:
  - RED: 写测试验证 `ALLOWED_VIDEO_MIME_TYPES` 包含 `video/mp4`, `video/webm`, `video/quicktime`, `video/x-matroska`
  - RED: 写测试验证 `video_mime_from_extension` 映射 mp4→video/mp4, webm→video/webm, mov→video/quicktime, mkv→video/x-matroska, avi→video/x-msvideo
  - RED: 写测试验证 `video_mime_from_magic` 检测 MP4 ftyp header 和 WebM EBML header
  - RED: 写测试验证 `validate_video_mime` 拒绝非视频 MIME 类型
  - GREEN: 添加 `ALLOWED_VIDEO_MIME_TYPES` 常量
  - GREEN: 实现 `video_mime_from_extension`、`video_mime_from_magic`、`validate_video_mime`
  - REFACTOR: 确保与现有 image MIME 函数风格一致
  **Must NOT do**:
  - 不修改现有 `ALLOWED_IMAGE_MIME_TYPES` 或 image MIME 函数
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3)
  - **Blocks**: Task 5
  - **Blocked By**: None
  **References**:
  - `src/multimodal.rs:8-14` — `ALLOWED_IMAGE_MIME_TYPES` 常量，视频版本复制此模式
  - `src/multimodal.rs:411-420` — `mime_from_extension` 函数，视频版本添加 mp4/webm/mov/mkv/avi 映射
  - `src/multimodal.rs:422-444` — `mime_from_magic` 函数，视频版本添加 ftyp (MP4) 和 EBML (WebM) magic bytes
  - `src/multimodal.rs:370-380` — `validate_mime` 函数，`validate_video_mime` 复制此模式
  **Acceptance Criteria**:
  - [ ] `video_mime_from_extension("mp4") == Some("video/mp4")`
  - [ ] `video_mime_from_extension("webm") == Some("video/webm")`
  - [ ] `video_mime_from_magic` 检测 MP4 ftyp header（bytes[4..8] == b"ftyp"）
  - [ ] `validate_video_mime("video/mp4")` 通过，`validate_video_mime("image/png")` 失败
  **QA Scenarios**:
  ```
  Scenario: Video MIME type validation
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw multimodal::tests::video_mime -- --nocapture 2>&1
      2. 检查输出包含 test result: ok
    Expected Result: 所有视频 MIME 测试通过
    Evidence: .sisyphus/evidence/task-4-video-mime.txt
  Scenario: Video magic byte detection
    Tool: Bash (cargo test)
    Steps:
      1. 写测试：构造 MP4 ftyp header bytes，验证 video_mime_from_magic 返回 video/mp4
      2. 写测试：构造 WebM EBML header bytes，验证返回 video/webm
    Expected Result: Magic byte 检测正确
    Evidence: .sisyphus/evidence/task-4-video-magic.txt
  ```
  **Commit**: YES (groups with Wave 1)
  - Message: `feat(multimodal): add video MIME type validation`
  - Files: `src/multimodal.rs`
  - Pre-commit: `cargo test multimodal`
### Wave 2 — Core Pipeline
- [ ] 5. multimodal.rs prepare_messages 视频处理 (TDD)
  **What to do**:
  - RED: 写测试验证 `prepare_messages_for_provider` 能处理包含 `[VIDEO:url]` 的消息
  - RED: 写测试验证视频数量超限时返回 `TooManyVideos` 错误
  - RED: 写测试验证视频 URL 直传（不做 base64 转换）— URL 保持原样
  - RED: 写测试验证混合 IMAGE + VIDEO 消息正确处理
  - GREEN: 扩展 `prepare_messages_for_provider` 处理视频 marker
  - GREEN: 视频走 URL pass-through：不下载、不 base64、不验证大小，直接保留 URL
  - GREEN: 添加 `TooManyVideos` error variant 到 `MultimodalError`
  - REFACTOR: 确保视频和图片路径清晰分离
  **Must NOT do**:
  - 不对视频做 base64 编码/归一化（视频文件太大）
  - 不下载视频内容
  - 不验证视频文件大小（URL 直传，由模型端验证）
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 6, 7, 8)
  - **Blocks**: Task 13
  - **Blocked By**: Tasks 1, 3, 4
  **References**:
  - `src/multimodal.rs:115-171` — `prepare_messages_for_provider` 函数，这是核心修改点。当前只处理 image marker，需要扩展处理 video marker
  - `src/multimodal.rs:22-51` — `MultimodalError` enum，需要添加 `TooManyVideos` variant
  - `src/multimodal.rs:160` — `compose_multimodal_message` 调用，视频需要类似的 compose 逻辑但保留 URL
  - `src/multimodal.rs:500-561` — 现有 prepare_messages 测试，视频测试必须覆盖相同场景
  **Acceptance Criteria**:
  - [ ] `[VIDEO:https://example.com/v.mp4]` 保留为 `[VIDEO:https://example.com/v.mp4]`（URL 不变）
  - [ ] 视频数量超过 `max_videos` 时返回错误
  - [ ] 图片+视频混合消息正确处理
  **QA Scenarios**:
  ```
  Scenario: Video URL pass-through
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw multimodal::tests::prepare_messages_video -- --nocapture 2>&1
    Expected Result: 视频 URL 保持原样，不做 base64 转换
    Evidence: .sisyphus/evidence/task-5-video-passthrough.txt
  Scenario: Too many videos rejected
    Tool: Bash (cargo test)
    Steps:
      1. 写测试：3个视频 marker + max_videos=2 → 验证返回 TooManyVideos 错误
    Expected Result: 超限时返回明确错误
    Evidence: .sisyphus/evidence/task-5-too-many-videos.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `feat(multimodal): add video URL pass-through in prepare_messages`
  - Files: `src/multimodal.rs`
  - Pre-commit: `cargo test multimodal`
- [ ] 6. multimodal.rs PreparedMessages 扩展 (TDD)
  **What to do**:
  - RED: 写测试验证 `PreparedMessages` 有 `contains_videos: bool` 字段
  - RED: 写测试验证 `prepare_messages_for_provider` 在有视频时设置 `contains_videos = true`
  - GREEN: 在 `PreparedMessages` struct 添加 `pub contains_videos: bool`
  - GREEN: 更新 `prepare_messages_for_provider` 返回值设置 `contains_videos`
  - REFACTOR: 确保所有现有调用点编译通过（添加 `contains_videos: false` 到现有构造）
  **Must NOT do**:
  - 不修改 `contains_images` 字段的行为
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 7, 8)
  - **Blocks**: Tasks 12, 13
  - **Blocked By**: Task 1
  **References**:
  - `src/multimodal.rs:16-20` — `PreparedMessages` struct，当前只有 `messages` 和 `contains_images`
  - `src/multimodal.rs:132-136` — 返回 `contains_images: false` 的路径，需要同步添加 `contains_videos: false`
  - `src/multimodal.rs:167-170` — 返回 `contains_images: true` 的路径
  - `src/gateway/ws.rs:116` — 调用 `prepare_messages_for_provider` 的地方，需要编译通过
  - `src/gateway/mod.rs:846` — 另一个调用点
  - `src/agent/loop_.rs:2049` — agent loop 中的调用点
  **Acceptance Criteria**:
  - [ ] `PreparedMessages` 有 `contains_videos` 字段
  - [ ] 有视频 marker 时 `contains_videos == true`
  - [ ] 无视频 marker 时 `contains_videos == false`
  - [ ] 所有现有调用点编译通过
  **QA Scenarios**:
  ```
  Scenario: PreparedMessages contains_videos flag
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw multimodal::tests -- --nocapture 2>&1
    Expected Result: 所有 multimodal 测试通过
    Evidence: .sisyphus/evidence/task-6-prepared-messages.txt
  Scenario: Compilation check
    Tool: Bash (cargo check)
    Steps:
      1. cargo check 2>&1
    Expected Result: 无编译错误
    Evidence: .sisyphus/evidence/task-6-compile-check.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `feat(multimodal): add contains_videos to PreparedMessages`
  - Files: `src/multimodal.rs`
  - Pre-commit: `cargo check`
- [ ] 7. compatible.rs VideoUrl MessagePart (TDD)
  **What to do**:
  - RED: 写测试验证 `MessagePart` enum 有 `VideoUrl` variant
  - RED: 写测试验证 `to_message_content` 解析 `[VIDEO:url]` marker 生成 `VideoUrl` part
  - RED: 写测试验证序列化为 `{"type": "video_url", "video_url": {"url": "..."}}`
  - RED: 写测试验证混合 IMAGE + VIDEO marker 生成正确的 Parts 数组
  - GREEN: 添加 `VideoUrl { video_url: VideoUrlPart }` 到 `MessagePart` enum
  - GREEN: 添加 `struct VideoUrlPart { url: String }`
  - GREEN: 扩展 `to_message_content` 解析 video marker
  - GREEN: 在 `OpenAiCompatibleProvider` 中设置 `video: true` 当 `supports_vision` 为 true 时
  - REFACTOR: 确保 serde tag/rename 与 ImageUrl 一致
  **Must NOT do**:
  - 不添加 fps、min_pixels、max_pixels 等 Qwen 特有字段
  - 不修改 ImageUrl 的行为
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 6, 8)
  - **Blocks**: Task 13
  - **Blocked By**: Tasks 2, 3
  **References**:
  - `src/providers/compatible.rs:330-340` — `MessagePart` enum 和 `ImageUrlPart` struct，VideoUrl 必须完全复制此模式
  - `src/providers/compatible.rs:323-328` — `MessageContent` enum，不需要修改
  - `src/providers/compatible.rs:882-907` — `to_message_content` 函数，需要扩展解析 video marker
  - `src/providers/compatible.rs:5` — `use crate::multimodal;` 已导入
  - `src/providers/compatible.rs:2301-2306` — 现有 multimodal 序列化测试，视频测试复制此模式
  **Acceptance Criteria**:
  - [ ] `MessagePart::VideoUrl` 序列化为 `{"type": "video_url", "video_url": {"url": "..."}}`
  - [ ] `to_message_content("user", "Look [VIDEO:https://v.mp4]")` 生成 Parts 含 VideoUrl
  - [ ] 混合 IMAGE + VIDEO 生成正确的 Parts 数组
  **QA Scenarios**:
  ```
  Scenario: VideoUrl serialization format
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw compatible::tests::video -- --nocapture 2>&1
    Expected Result: VideoUrl 序列化格式正确
    Evidence: .sisyphus/evidence/task-7-video-url-serialize.txt
  Scenario: Mixed image and video content
    Tool: Bash (cargo test)
    Steps:
      1. 写测试：内容包含 [IMAGE:a.png] 和 [VIDEO:v.mp4]
      2. 验证 Parts 数组包含 Text + ImageUrl + VideoUrl
    Expected Result: 混合内容正确序列化
    Evidence: .sisyphus/evidence/task-7-mixed-content.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `feat(compatible): add VideoUrl content part for video multimodal`
  - Files: `src/providers/compatible.rs`
  - Pre-commit: `cargo test compatible`
- [ ] 8. openrouter.rs VideoUrl MessagePart (TDD)
  **What to do**:
  - RED: 写测试验证 `MessagePart` enum 有 `VideoUrl` variant（与 compatible.rs 一致）
  - RED: 写测试验证 `to_message_content` 解析 `[VIDEO:url]` marker
  - RED: 写测试验证序列化为 `{"type": "video_url", "video_url": {"url": "..."}}`
  - GREEN: 添加 `VideoUrl { video_url: VideoUrlPart }` 到 `MessagePart` enum
  - GREEN: 添加 `struct VideoUrlPart { url: String }`
  - GREEN: 扩展 `to_message_content` 解析 video marker
  - REFACTOR: 确保与 compatible.rs 的实现完全对称
  **Must NOT do**:
  - 不修改 ImageUrl 的行为
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with Tasks 5, 6, 7)
  - **Blocks**: Task 13
  - **Blocked By**: Tasks 2, 3
  **References**:
  - `src/providers/openrouter.rs:35-45` — `MessagePart` enum 和 `ImageUrlPart` struct，与 compatible.rs 结构对称
  - `src/providers/openrouter.rs:250-274` — `to_message_content` 函数，需要扩展解析 video marker
  - `src/providers/openrouter.rs:880-885` — 现有 multimodal 序列化测试，视频测试复制此模式
  - `src/providers/compatible.rs:330-340` — compatible.rs 的 MessagePart 作为参考，确保两边一致
  **Acceptance Criteria**:
  - [ ] `MessagePart::VideoUrl` 序列化格式与 compatible.rs 一致
  - [ ] `to_message_content("user", "[VIDEO:https://v.mp4]")` 生成 VideoUrl part
  **QA Scenarios**:
  ```
  Scenario: OpenRouter VideoUrl serialization
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw openrouter::tests::video -- --nocapture 2>&1
    Expected Result: VideoUrl 序列化格式正确
    Evidence: .sisyphus/evidence/task-8-openrouter-video.txt
  ```
  **Commit**: YES (groups with Wave 2)
  - Message: `feat(openrouter): add VideoUrl content part for video multimodal`
  - Files: `src/providers/openrouter.rs`
  - Pre-commit: `cargo test openrouter`
### Wave 3 — Channel Integration
- [ ] 9. telegram.rs parse_attachment_metadata 视频支持 (TDD)
  **What to do**:
  - RED: 写测试验证 `parse_attachment_metadata` 能识别 `message.video` 字段
  - RED: 写测试验证返回 `IncomingAttachmentKind::Video` 和正确的 file_id/file_name/file_size
  - RED: 写测试验证视频消息有 caption 时正确提取
  - GREEN: 在 `parse_attachment_metadata` 函数中添加 `message.video` 分支
  - GREEN: 添加 `IncomingAttachmentKind::Video` variant（如果不存在）
  - REFACTOR: 确保与 Photo/Document 分支风格一致
  **Must NOT do**:
  - 不修改现有 Photo/Document 解析逻辑
  - 不处理视频转码或抽帧
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 10, 11)
  - **Blocks**: Task 10
  - **Blocked By**: None
  **References**:
  - `src/channels/telegram.rs:862-908` — `parse_attachment_metadata` 函数，当前只处理 document 和 photo
  - `src/channels/telegram.rs:33` — `IncomingAttachmentKind` enum，需要添加 Video variant
  - `src/channels/telegram.rs:4025-4127` — 现有 parse_attachment_metadata 测试，视频测试复制此模式
  - Telegram Bot API video message 结构: `{"video": {"file_id": "...", "file_name": "video.mp4", "file_size": 1234567, "duration": 30, "mime_type": "video/mp4"}}`
  **Acceptance Criteria**:
  - [ ] `parse_attachment_metadata` 识别 `message.video` 字段
  - [ ] 返回 `IncomingAttachmentKind::Video` + 正确的 file_id
  - [ ] 视频 caption 正确提取
  **QA Scenarios**:
  ```
  Scenario: Parse video attachment metadata
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw telegram::tests::parse_attachment_metadata_detects_video -- --nocapture 2>&1
    Expected Result: 视频附件元数据正确解析
    Evidence: .sisyphus/evidence/task-9-telegram-video-parse.txt
  ```
  **Commit**: YES (groups with Wave 3)
  - Message: `feat(telegram): parse incoming video attachment metadata`
  - Files: `src/channels/telegram.rs`
  - Pre-commit: `cargo test telegram`
- [ ] 10. telegram.rs format_attachment_content 视频 marker (TDD)
  **What to do**:
  - RED: 写测试验证 `format_attachment_content(IncomingAttachmentKind::Video, ...)` 生成 `[VIDEO:/path/to/video.mp4]`
  - RED: 写测试验证视频文件扩展名检测函数 `is_video_extension`
  - GREEN: 添加 `is_video_extension` 函数（mp4, webm, mov, mkv, avi）
  - GREEN: 扩展 `format_attachment_content` 处理 `IncomingAttachmentKind::Video`
  - REFACTOR: 确保与 Photo → `[IMAGE:]` 的模式一致
  **Must NOT do**:
  - 不修改 Photo → `[IMAGE:]` 的逻辑
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 9, 11)
  - **Blocks**: Task 13
  - **Blocked By**: Tasks 3, 9
  **References**:
  - `src/channels/telegram.rs:174-187` — `format_attachment_content` 函数，当前只处理 Photo 和 fallback Document
  - `src/channels/telegram.rs:156-166` — `is_image_extension` 函数，`is_video_extension` 复制此模式
  - `src/channels/telegram.rs:4151-4188` — 现有 format_attachment_content 测试
  **Acceptance Criteria**:
  - [ ] `format_attachment_content(Video, "clip.mp4", path)` 返回 `[VIDEO:/path/clip.mp4]`
  - [ ] `is_video_extension` 识别 mp4, webm, mov, mkv, avi
  - [ ] 非视频扩展名的 Video kind 回退到 Document 格式
  **QA Scenarios**:
  ```
  Scenario: Video attachment generates VIDEO marker
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw telegram::tests::format_attachment_content_video -- --nocapture 2>&1
    Expected Result: 视频附件生成 [VIDEO:] marker
    Evidence: .sisyphus/evidence/task-10-video-marker.txt
  ```
  **Commit**: YES (groups with Wave 3)
  - Message: `feat(telegram): generate VIDEO markers for incoming video`
  - Files: `src/channels/telegram.rs`
  - Pre-commit: `cargo test telegram`
- [ ] 11. whatsapp.rs 视频消息解析 (TDD)
  **What to do**:
  - RED: 写测试验证 WhatsApp 视频消息（`type: "video"`）能被解析而非跳过
  - RED: 写测试验证视频消息生成 `[VIDEO:url]` marker（使用 media download URL）
  - GREEN: 在 `parse_webhook_payload` 中添加 `type: "video"` 分支
  - GREEN: 下载视频文件到 workspace，生成 `[VIDEO:/path]` marker
  - REFACTOR: 确保与 text 消息处理路径风格一致
  **Must NOT do**:
  - 不做视频转码或抽帧
  - 不修改现有 text 消息处理逻辑
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with Tasks 9, 10)
  - **Blocks**: Task 13
  - **Blocked By**: Task 3
  **References**:
  - `src/channels/whatsapp.rs:110-119` — 当前跳过非 text 消息的代码，需要添加 video 分支
  - `src/channels/whatsapp.rs:739-778` — 现有 `whatsapp_parse_video_message_skipped` 测试，需要改为验证视频被处理
  - WhatsApp video webhook 结构: `{"type": "video", "video": {"id": "media_id", "mime_type": "video/mp4"}}`
  **Acceptance Criteria**:
  - [ ] WhatsApp 视频消息不再被跳过
  - [ ] 视频消息生成 `[VIDEO:...]` marker
  - [ ] 现有 text 消息处理不受影响
  **QA Scenarios**:
  ```
  Scenario: WhatsApp video message parsed
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw whatsapp::tests::video -- --nocapture 2>&1
    Expected Result: 视频消息被正确解析
    Evidence: .sisyphus/evidence/task-11-whatsapp-video.txt
  ```
  **Commit**: YES (groups with Wave 3)
  - Message: `feat(whatsapp): parse incoming video messages`
  - Files: `src/channels/whatsapp.rs`
  - Pre-commit: `cargo test whatsapp`
### Wave 4 — Integration
- [ ] 12. agent/loop_.rs 视频 capability 检查 (TDD)
  **What to do**:
  - RED: 写测试验证当 provider 不支持 video 时，视频 marker 被过滤/报警
  - RED: 写测试验证当 provider 支持 video 时，视频 marker 正常传递
  - GREEN: 在 agent loop 的 multimodal 处理路径中添加 `supports_video()` 检查
  - GREEN: 当 `contains_videos == true` 且 provider 不支持 video 时，log warning 并移除视频 marker
  - REFACTOR: 确保与现有 `supports_vision()` 检查风格一致
  **Must NOT do**:
  - 不修改现有 vision 检查逻辑
  - 不阻断消息发送（仅移除视频 marker + warning）
  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4 (with Task 13)
  - **Blocks**: Task 13
  - **Blocked By**: Tasks 2, 6
  **References**:
  - `src/agent/loop_.rs:2006-2049` — multimodal 处理路径，`count_image_markers` 和 `prepare_messages_for_provider` 调用
  - `src/agent/loop_.rs:3401` — `count_image_markers` 在 request validation 中的使用
  - `src/agent/loop_.rs:2959,3080,3312` — `&config.multimodal` 传递点
  - `src/providers/traits.rs:386-389` — `supports_vision()` 方法，`supports_video()` 检查复制此模式
  **Acceptance Criteria**:
  - [ ] provider 不支持 video 时，视频 marker 被移除，log warning
  - [ ] provider 支持 video 时，视频 marker 正常传递
  - [ ] 现有 image 处理不受影响
  **QA Scenarios**:
  ```
  Scenario: Video capability check in agent loop
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p zeroclaw agent::tests -- video --nocapture 2>&1
    Expected Result: 视频 capability 检查测试通过
    Evidence: .sisyphus/evidence/task-12-agent-video-check.txt
  Scenario: Full compilation check
    Tool: Bash (cargo check)
    Steps:
      1. cargo check 2>&1
    Expected Result: 无编译错误
    Evidence: .sisyphus/evidence/task-12-compile-check.txt
  ```
  **Commit**: YES (groups with Wave 4)
  - Message: `feat(agent): add video capability check in agent loop`
  - Files: `src/agent/loop_.rs`
  - Pre-commit: `cargo check`
- [ ] 13. 集成测试 — 端到端视频 marker 流 (TDD)
  **What to do**:
  - RED: 写集成测试验证完整流程：`[VIDEO:https://example.com/v.mp4]` → parse → prepare → VideoUrl content part
  - RED: 写测试验证混合 IMAGE + VIDEO 消息的完整流程
  - RED: 写测试验证 provider 不支持 video 时的降级行为
  - RED: 写回归测试确认现有 image 流程不受影响
  - GREEN: 确保所有集成测试通过
  - REFACTOR: 清理测试代码，确保测试命名一致
  **Must NOT do**:
  - 不引入新的外部依赖
  - 不修改任何非测试代码
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4 (after Task 12)
  - **Blocks**: F1-F4
  - **Blocked By**: Tasks 5, 7, 10, 12
  **References**:
  - `src/multimodal.rs:446-569` — 现有 multimodal 测试模块，集成测试复制此模式
  - `src/providers/compatible.rs:2280-2310` — 现有 multimodal 序列化测试
  - `src/providers/openrouter.rs:870-890` — OpenRouter multimodal 测试
  - `src/channels/telegram.rs:4025-4428` — Telegram 附件测试
  **Acceptance Criteria**:
  - [ ] `cargo test` 全部通过（包括新增和现有测试）
  - [ ] `cargo clippy --all-targets -- -D warnings` 无警告
  - [ ] `cargo fmt --all -- --check` 格式正确
  - [ ] 现有 image multimodal 测试无回归
  **QA Scenarios**:
  ```
  Scenario: Full test suite pass
    Tool: Bash (cargo test)
    Steps:
      1. cargo test 2>&1
      2. 检查输出包含 test result: ok
    Expected Result: 所有测试通过，0 failures
    Evidence: .sisyphus/evidence/task-13-full-test.txt
  Scenario: Clippy + fmt clean
    Tool: Bash
    Steps:
      1. cargo clippy --all-targets -- -D warnings 2>&1
      2. cargo fmt --all -- --check 2>&1
    Expected Result: 无警告，无格式问题
    Evidence: .sisyphus/evidence/task-13-lint-fmt.txt
  Scenario: Image regression check
    Tool: Bash (cargo test)
    Steps:
      1. cargo test multimodal::tests::parse_image -- --nocapture 2>&1
      2. cargo test multimodal::tests::prepare_messages_normalizes -- --nocapture 2>&1
    Expected Result: 所有现有 image 测试仍然通过
    Evidence: .sisyphus/evidence/task-13-image-regression.txt
  ```
  **Commit**: YES
  - Message: `test(multimodal): add integration tests for video pipeline`
  - Files: `src/multimodal.rs`, `src/providers/compatible.rs`
  - Pre-commit: `cargo test`

## Final Verification Wave

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test` in WSL2. Review all changed files for: `as any` equivalent unsafe casts, empty error handling, `todo!()` or `unimplemented!()` in non-test code, unused imports. Check AI slop: excessive comments, over-abstraction, generic names.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real QA** — `unspecified-high`
  Start from clean state. Run full test suite: `cargo test 2>&1`. Verify all new video-related tests pass. Check that existing image multimodal tests still pass (no regression). Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`.
  Output: `Tests [N/N pass] | Clippy [PASS/FAIL] | Fmt [PASS/FAIL] | Regression [CLEAN/N issues] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (git diff). Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance: no base64 video encoding, no Ollama changes, no Anthropic/Gemini changes, no frame extraction, no new dependencies. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Scope [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

- **Wave 1**: `feat(multimodal): add video config, capability flag, and marker parsing` — config/schema.rs, providers/traits.rs, multimodal.rs
- **Wave 2**: `feat(providers): add VideoUrl content part for video multimodal` — compatible.rs, openrouter.rs, multimodal.rs
- **Wave 3**: `feat(channels): handle incoming video messages as VIDEO markers` — telegram.rs, whatsapp.rs
- **Wave 4**: `feat(agent): integrate video capability check in agent loop` — agent/loop_.rs
- **Final**: `test(multimodal): add integration tests for video pipeline` — test files

---

## Success Criteria

### Verification Commands
```bash
cargo test                                    # Expected: all tests pass
cargo clippy --all-targets -- -D warnings     # Expected: no warnings
cargo fmt --all -- --check                    # Expected: no formatting issues
cargo test multimodal                         # Expected: all multimodal tests pass (image + video)
cargo test video                              # Expected: all video-specific tests pass
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass in WSL2
- [ ] No regression in existing image multimodal tests
- [ ] `[VIDEO:url]` marker correctly parsed and dispatched as `video_url` content part
