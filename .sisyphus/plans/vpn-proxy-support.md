# VPN/Proxy Smart Routing Support for ZeroClaw

## TL;DR

> **Quick Summary**: Add embedded VPN proxy support to ZeroClaw using clash-lib (clash-rs) as the proxy runtime and subconverter-rs for Clash subscription parsing. Agent gets full control over VPN lifecycle via a new `vpn_control` tool, with latency-based node selection, 30s health checks, auto-failover, and domestic domain bypass.
> 
> **Deliverables**:
> - New `src/vpn/` module (behind `--features vpn` cargo feature flag)
> - `[vpn]` config section with `clash_proxy_url` subscription URL
> - `vpn_control` tool for agent to manage VPN (on/off, node list, switch, status, refresh)
> - Auto-failover: health check every 30s, auto-switch node or disable on failure
> - Domestic bypass: built-in domain list (fast path) + IP geo API fallback (uapis.cn) + agent-configurable
> - Disk-cached node list at `~/.zeroclaw/state/vpn/`
> - Fix existing proxy gaps (web_search_tool, OAuth clients)
> 
> **Estimated Effort**: Large
> **Parallel Execution**: YES - 4 waves
> **Critical Path**: Task 1 → Task 3 → Task 5 → Task 7 → Task 9 → Task 11 → Task 13 → F1-F4

---

## Context

### Original Request
用户需要为 ZeroClaw 添加 VPN 支持：
- 支持 ZeroClaw 自己切换 VPN 的开启和切节点
- 如果出现 VPN 导致的断网，VPN 自动切换到可用的节点或者关闭
- 使用 subconverter-rs 解析订阅
- 在配置中暴露 `clash_proxy_url` 供用户设置
- Agent 不使用的联网功能都不过 VPN
- Agent 使用的联网功能可选择性地走 VPN proxy
- 对于国内服务（飞书/百度/B站等）不使用 VPN

### Interview Summary
**Key Discussions**:
- subconverter-rs: 作为 crate 依赖嵌入（GPL-3.0 许可证已接受）
- 代理运行时: clash-lib (clash-rs) 嵌入式 Rust 库，Apache-2.0
- 节点选择: 延迟优先（定期测速，选最低延迟可用节点）
- Agent 控制: 完整控制（开关/节点列表/切换/状态/刷新订阅）
- 国内绕过: 内置域名列表（快速路径）+ IP 归属地 API 兜底（uapis.cn）+ agent 运行时可配置
- Feature flag: `--features vpn` 保护
- 节点持久化: 磁盘缓存 `~/.zeroclaw/state/vpn/`
- 健康检查: 30 秒间隔
- 测试策略: TDD（先测试后实现）

**Research Findings**:
- ZeroClaw 已有成熟的 ProxyConfig 系统（per-service routing, scope modes, runtime hot-swap）
- reqwest 已启用 socks feature
- 所有主要 provider/channel 使用 proxy-aware client builders
- GAP: `web_search_tool.rs` 未使用 proxy integration
- GAP: OAuth blocking clients (MiniMax, Qwen) 未应用 proxy
- clash-lib 支持从字符串加载配置，暴露本地 SOCKS5/HTTP 监听端口

### Metis Review
**Identified Gaps** (addressed):
- 架构方案未决 → 已确认: clash-lib 嵌入式运行时 + subconverter-rs 订阅解析
- subconverter-rs GPL-3.0 许可证冲突 → 用户已接受 GPL-3.0 for vpn feature
- 代理链冲突（[proxy] + [vpn]）→ VPN 通过 set_runtime_proxy_config() 更新，与现有 proxy 系统集成
- 所有节点失败时的回退策略 → 禁用 VPN，恢复直连
- 订阅 URL 拉取失败 → 使用磁盘缓存的节点列表
- 节点切换期间的 in-flight 请求 → 下次请求生效（client cache 自动失效）

---

## Work Objectives

### Core Objective
为 ZeroClaw 添加嵌入式 VPN 代理支持，使 agent 能够自主管理 VPN 连接，实现智能路由（国内直连、国外走代理），并在节点故障时自动切换。

### Concrete Deliverables
- `src/vpn/mod.rs` — 模块入口和公共 API
- `src/vpn/subscription.rs` — Clash 订阅解析（via subconverter-rs）
- `src/vpn/runtime.rs` — clash-lib 代理运行时管理
- `src/vpn/health.rs` — 节点健康检查和延迟测试
- `src/vpn/node_manager.rs` — 节点选择、切换、持久化
- `src/vpn/bypass.rs` — 国内绕过判断（域名列表 + IP 归属地 API 兜底）
- `src/tools/vpn_control.rs` — Agent VPN 控制工具
- `src/config/schema.rs` — `[vpn]` 配置段
- Cargo.toml `vpn` feature flag
- 修复 `web_search_tool.rs` 和 OAuth clients 的 proxy gap

### Definition of Done
- [ ] `cargo build --release` (无 features) 编译不包含 VPN 代码
- [ ] `cargo build --release --features vpn` 编译通过
- [ ] `cargo test --features vpn` 全部通过
- [ ] `cargo clippy --all-targets --features vpn -- -D warnings` 通过
- [ ] 现有 `proxy_config` tool 测试不受影响
- [ ] 无 `[vpn]` 段的 config.toml 正常反序列化

### Must Have
- clash-lib 嵌入式代理运行时（本地 SOCKS5/HTTP 监听）
- subconverter-rs 订阅解析
- `clash_proxy_url` 配置项
- 30s 健康检查 + 自动切换
- 延迟优先节点选择
- 国内绕过：域名列表快速路径 + IP 归属地 API 兜底（uapis.cn）
- `vpn_control` tool（get/set/list_nodes/switch_node/refresh/status）
- 节点列表磁盘缓存
- `--features vpn` feature flag 隔离
- 所有节点失败时自动禁用 VPN 恢复直连

### Must NOT Have (Guardrails)
- 不在进程内实现 VMess/VLESS/Trojan 协议处理（由 clash-lib 处理）
- 不在现有模块中散布 `#[cfg(feature = "vpn")]`（仅在注册点使用）
- 不破坏现有 `[proxy]` 配置行为
- 不增加无 `--features vpn` 时的二进制大小
- 不添加 GUI/dashboard
- 不实现 DNS-over-proxy
- 不实现非 ZeroClaw 应用的 split tunneling
- 不嵌入 GeoIP 数据库（使用在线 IP 归属地 API 兜底）
- 不支持多个同时活跃节点（单活 + failover）
- 暂不支持 V2Ray/SIP008/base64 订阅格式（仅 Clash YAML）
- 暂不支持代理链（VPN through existing proxy）

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed. No exceptions.

### Test Decision
- **Infrastructure exists**: YES (cargo test)
- **Automated tests**: TDD (RED → GREEN → REFACTOR)
- **Framework**: cargo test (Rust built-in)
- **Each task follows**: Write failing test → Implement minimal code → Refactor

### QA Policy
Every task MUST include agent-executed QA scenarios.
Evidence saved to `.sisyphus/evidence/task-{N}-{scenario-slug}.{ext}`.

- **Library/Module**: Use Bash (cargo test) — Run tests, compare output
- **Config**: Use Bash — Parse config, verify schema
- **Tool**: Use Bash (cargo test + curl if gateway running) — Invoke tool, verify response
- **Integration**: Use Bash — Full flow test with mock subscription

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — config + types + feature flag + proxy gap fixes):
├── Task 1: Feature flag setup + Cargo.toml dependencies [quick]
├── Task 2: VPN config schema + types [quick]
├── Task 3: Fix web_search_tool.rs proxy gap [quick]
├── Task 4: Fix OAuth blocking clients proxy gap [quick]
└── Task 5: Domestic bypass (domain list + IP geo API fallback) [unspecified-high]

Wave 2 (Core modules — subscription + runtime + node manager):
├── Task 6: Subscription parser (subconverter-rs integration) [deep]
├── Task 7: clash-lib runtime manager [deep]
├── Task 8: Node persistence (disk cache) [unspecified-high]
└── Task 9: Health checker + latency tester [deep]

Wave 3 (Integration — node manager + tool + proxy bridge):
├── Task 10: Node manager (selection strategy + failover logic) [deep]
├── Task 11: VPN ↔ ProxyConfig bridge (runtime proxy integration) [deep]
└── Task 12: vpn_control tool implementation [unspecified-high]

Wave 4 (Wiring + verification):
├── Task 13: Module registration + tool wiring + startup/shutdown [unspecified-high]
└── Task 14: Integration tests (full flow) [deep]

Wave FINAL (After ALL tasks — independent review, 4 parallel):
├── Task F1: Plan compliance audit (oracle)
├── Task F2: Code quality review (unspecified-high)
├── Task F3: Real manual QA (unspecified-high)
└── Task F4: Scope fidelity check (deep)

Critical Path: Task 1 → Task 6 → Task 7 → Task 9 → Task 10 → Task 11 → Task 13 → Task 14 → F1-F4
Parallel Speedup: ~60% faster than sequential
Max Concurrent: 5 (Wave 1)
```

### Dependency Matrix

| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | — | 2, 5, 6, 7, 8, 9 |
| 2 | 1 | 5, 6, 7, 8, 9, 10, 11, 12, 13 |
| 3 | — | 14 |
| 4 | — | 14 |
| 5 | 1, 2 | 10, 11 |
| 6 | 1, 2 | 8, 10 |
| 7 | 1, 2 | 9, 10, 11 |
| 8 | 1, 2, 6 | 10 |
| 9 | 1, 2, 7 | 10 |
| 10 | 5, 6, 7, 8, 9 | 11, 12 |
| 11 | 7, 10 | 12, 13 |
| 12 | 10, 11 | 13 |
| 13 | 2, 11, 12 | 14 |
| 14 | 3, 4, 13 | F1-F4 |

### Agent Dispatch Summary

- **Wave 1**: 5 tasks — T1,T2,T5 → `quick`, T3 → `quick`, T4 → `quick`
- **Wave 2**: 4 tasks — T6 → `deep`, T7 → `deep`, T8 → `unspecified-high`, T9 → `deep`
- **Wave 3**: 3 tasks — T10 → `deep`, T11 → `deep`, T12 → `unspecified-high`
- **Wave 4**: 2 tasks — T13 → `unspecified-high`, T14 → `deep`
- **FINAL**: 4 tasks — F1 → `oracle`, F2 → `unspecified-high`, F3 → `unspecified-high`, F4 → `deep`

---

## TODOs

- [x] 1. Feature Flag Setup + Cargo.toml Dependencies

  **What to do**:
  - Add `vpn` feature flag to `Cargo.toml` with dependencies: `clash-lib`, `subconverter` (subconverter-rs crate)
  - Add conditional compilation: `[target.'cfg(feature = "vpn")'.dependencies]` section
  - Create empty `src/vpn/mod.rs` with `#![cfg(feature = "vpn")]` gate
  - Register `pub mod vpn;` in `src/lib.rs` behind `#[cfg(feature = "vpn")]`
  - TDD: Write test that verifies `cargo check` passes with and without `--features vpn`

  **Must NOT do**:
  - Do not add any VPN logic yet — only scaffolding
  - Do not add `#[cfg(feature = "vpn")]` to any existing module beyond `src/lib.rs`

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Single-file Cargo.toml edit + minimal scaffolding
  - **Skills**: []
  - **Skills Evaluated but Omitted**:
    - `git-master`: Not needed — no git operations in this task

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 2, 3, 4, 5)
  - **Blocks**: Tasks 2, 5, 6, 7, 8, 9
  - **Blocked By**: None (can start immediately)

  **References**:
  **Pattern References**:
  - `Cargo.toml:60-61` — `fantoccini` optional dependency pattern (`optional = true, default-features = false`)
  - `Cargo.toml:32` — `matrix-sdk` optional dependency with feature gating
  - `src/lib.rs` — Module registration pattern, look for `#[cfg(feature = "..." )]` guards

  **API/Type References**:
  - clash-rs GitHub: `https://github.com/Watfaq/clash-rs` — check `clash-lib/Cargo.toml` for crate name and features
  - subconverter-rs: `https://github.com/lonelam/subconverter-rs` — check crate name on crates.io

  **WHY Each Reference Matters**:
  - Cargo.toml optional deps: Follow exact same pattern for `clash-lib` and `subconverter` to keep binary small without feature
  - lib.rs: Must match existing conditional module registration pattern

  **Acceptance Criteria**:
  - [ ] `cargo check` passes (no vpn feature — baseline)
  - [ ] `cargo check --features vpn` passes
  - [ ] `src/vpn/mod.rs` exists and is empty except for module-level doc comment
  - [ ] `cargo build --release` does NOT compile any clash-lib or subconverter code

  **QA Scenarios:**
  ```
  Scenario: Feature flag isolation — no VPN code without flag
    Tool: Bash
    Preconditions: Clean build
    Steps:
      1. Run `cargo build --release 2>&1`
      2. Verify output does NOT contain "clash" or "subconverter" in compilation lines
      3. Run `cargo build --release --features vpn 2>&1`
      4. Verify output DOES contain "clash" or "subconverter" in compilation lines
    Expected Result: VPN deps only compiled with --features vpn
    Failure Indicators: clash-lib compiled without feature flag
    Evidence: .sisyphus/evidence/task-1-feature-flag-isolation.txt

  Scenario: Module registration compiles
    Tool: Bash
    Preconditions: Feature flag added
    Steps:
      1. Run `cargo check --features vpn`
      2. Verify exit code 0
    Expected Result: Clean compilation
    Failure Indicators: Compilation errors
    Evidence: .sisyphus/evidence/task-1-module-registration.txt
  ```

  **Commit**: YES
  - Message: `feat(vpn): add vpn feature flag and dependencies`
  - Files: `Cargo.toml`, `src/vpn/mod.rs`, `src/lib.rs`
  - Pre-commit: `cargo check && cargo check --features vpn`

- [x] 2. VPN Config Schema + Types

  **What to do**:
  - Add `VpnConfig` struct to `src/config/schema.rs` behind `#[cfg(feature = "vpn")]`
  - Fields: `enabled: bool`, `clash_proxy_url: Option<String>` (subscription URL), `listen_port: u16` (default 7891), `health_check_interval_secs: u64` (default 30), `auto_failover: bool` (default true), `domestic_bypass_enabled: bool` (default true), `domestic_bypass_extra: Vec<String>` (user-added domains), `node_cache_path: Option<PathBuf>` (default `~/.zeroclaw/state/vpn/`), `subscription_refresh_interval_secs: u64` (default 3600), `max_latency_ms: u64` (default 3000)
  - Add `pub vpn: VpnConfig` field to `Config` struct (behind `#[cfg(feature = "vpn")]`)
  - Implement `Default` for `VpnConfig`
  - Add validation in `Config::validate()` — if vpn.enabled, clash_proxy_url must be set
  - Add env override: `ZEROCLAW_VPN_CLASH_PROXY_URL`, `ZEROCLAW_VPN_ENABLED`
  - TDD: Write tests for config deserialization (with/without `[vpn]` section), validation, env overrides

  **Must NOT do**:
  - Do not implement any VPN logic — only config schema
  - Do not break existing config.toml deserialization (missing `[vpn]` must work)

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Config schema addition follows well-established patterns in schema.rs
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (after Task 1)
  - **Parallel Group**: Wave 1 (with Tasks 1, 3, 4, 5)
  - **Blocks**: Tasks 5, 6, 7, 8, 9, 10, 11, 12, 13
  - **Blocked By**: Task 1

  **References**:
  **Pattern References**:
  - `src/config/schema.rs:1134-1172` — `ProxyConfig` struct pattern (enabled, URLs, scope, Default impl)
  - `src/config/schema.rs:66-215` — `Config` struct field registration pattern (serde default, doc comments)
  - `src/config/schema.rs:4326-4393` — Env override pattern for proxy (`ZEROCLAW_PROXY_*`)
  - `src/config/schema.rs:4109-4110` — Validation delegation pattern (`self.proxy.validate()?`)

  **API/Type References**:
  - `src/config/schema.rs:1122-1132` — `ProxyScope` enum as example of config enum with serde rename

  **WHY Each Reference Matters**:
  - ProxyConfig: Exact pattern to follow for VpnConfig (struct shape, Default, validate, env overrides)
  - Config struct: Where to add the new `vpn` field with `#[serde(default)]`
  - Env override section: Pattern for `ZEROCLAW_VPN_*` environment variable handling

  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn config` passes
  - [ ] Config without `[vpn]` section deserializes successfully
  - [ ] Config with `[vpn]` section deserializes correctly
  - [ ] `vpn.enabled = true` without `clash_proxy_url` fails validation
  - [ ] `ZEROCLAW_VPN_CLASH_PROXY_URL` env override works

  **QA Scenarios:**
  ```
  Scenario: Backward-compatible config deserialization
    Tool: Bash
    Preconditions: Existing config.toml without [vpn] section
    Steps:
      1. Run `cargo test --features vpn config_without_vpn_section_deserializes`
      2. Verify test passes
    Expected Result: Config loads with VpnConfig::default()
    Failure Indicators: Deserialization error on missing [vpn]
    Evidence: .sisyphus/evidence/task-2-backward-compat.txt

  Scenario: Validation rejects enabled without URL
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn vpn_enabled_without_url_fails_validation`
      2. Verify test passes
    Expected Result: Validation error mentioning clash_proxy_url
    Failure Indicators: Validation passes when it shouldn't
    Evidence: .sisyphus/evidence/task-2-validation.txt
  ```

  **Commit**: YES
  - Message: `feat(vpn): add VPN config schema and types`
  - Files: `src/config/schema.rs`
  - Pre-commit: `cargo test --features vpn`

- [x] 3. Fix web_search_tool.rs Proxy Gap

  **What to do**:
  - Add proxy integration to `web_search_tool.rs` at lines 38-41 and 114-116
  - Replace direct `reqwest::Client::builder()` with `apply_runtime_proxy_to_builder(builder, "tool.web_search")`
  - Add `"tool.web_search"` to `SUPPORTED_PROXY_SERVICE_KEYS` in `src/config/schema.rs`
  - TDD: Write test verifying web_search client uses proxy-aware builder

  **Must NOT do**:
  - Do not change web search behavior — only add proxy support
  - Do not modify any other tool

  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Two-line fix in existing file + one key addition
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 4, 5)
  - **Blocks**: Task 14
  - **Blocked By**: None (can start immediately)

  **References**:
  **Pattern References**:
  - `src/tools/http_request.rs:127` — Correct proxy integration pattern: `apply_runtime_proxy_to_builder(builder, "tool.http_request")`
  - `src/config/schema.rs:16-45` — `SUPPORTED_PROXY_SERVICE_KEYS` array where to add `"tool.web_search"`

  **API/Type References**:
  - `src/config/mod.rs` — `apply_runtime_proxy_to_builder` function signature

  **WHY Each Reference Matters**:
  - http_request.rs: Exact pattern to copy for web_search_tool.rs
  - SUPPORTED_PROXY_SERVICE_KEYS: Must add new key or proxy won't apply

  **Acceptance Criteria**:
  - [ ] `cargo test` passes (no feature flag needed — this is a bug fix)
  - [ ] `web_search_tool.rs` uses `apply_runtime_proxy_to_builder`
  - [ ] `"tool.web_search"` in SUPPORTED_PROXY_SERVICE_KEYS

  **QA Scenarios:**
  ```
  Scenario: Web search tool uses proxy-aware client
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `grep -n 'apply_runtime_proxy_to_builder' src/tools/web_search_tool.rs`
      2. Verify at least 2 matches (for both client creation points)
      3. Run `grep 'tool.web_search' src/config/schema.rs`
      4. Verify match in SUPPORTED_PROXY_SERVICE_KEYS
      5. Run `cargo test`
      6. Verify all tests pass
    Expected Result: Proxy integration present, tests green
    Failure Indicators: grep returns 0 matches, or tests fail
    Evidence: .sisyphus/evidence/task-3-proxy-gap-fix.txt
  ```

  **Commit**: YES
  - Message: `fix(tools): add proxy integration to web_search_tool`
  - Files: `src/tools/web_search_tool.rs`, `src/config/schema.rs`
  - Pre-commit: `cargo test`

---
- [x] 4. Fix OAuth Blocking Clients Proxy Gap
  **What to do**:
  - In `src/providers/mod.rs`, find OAuth blocking client creation (MiniMax ~line 528, Qwen ~line 356)
  - Apply proxy to blocking clients: build `reqwest::blocking::Client` with proxy from `runtime_proxy_config()`
  - Since `apply_runtime_proxy_to_builder` is async-only, create a blocking equivalent or manually apply proxy URLs
  - TDD: Write test verifying OAuth blocking clients respect proxy config
  **Must NOT do**:
  - Do not change OAuth flow logic — only add proxy support to HTTP client creation
  - Do not convert blocking clients to async
  **Recommended Agent Profile**:
  - **Category**: `quick`
    - Reason: Small targeted fix in one file
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3, 5)
  - **Blocks**: Task 14
  - **Blocked By**: None (can start immediately)
  **References**:
  **Pattern References**:
  - `src/providers/mod.rs:356-360` — Qwen OAuth blocking client creation (no proxy)
  - `src/providers/mod.rs:528-532` — MiniMax OAuth blocking client creation (no proxy)
  - `src/config/schema.rs:1255-1309` — `apply_to_reqwest_builder` method showing how proxy URLs are applied
  **WHY Each Reference Matters**:
  - mod.rs OAuth sections: Exact locations to fix
  - apply_to_reqwest_builder: Logic to replicate for blocking client (same proxy URL application, different builder type)
  **Acceptance Criteria**:
  - [ ] `cargo test` passes
  - [ ] OAuth blocking clients in `providers/mod.rs` apply proxy URLs
  **QA Scenarios:**
  ```
  Scenario: OAuth clients use proxy-aware builders
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `grep -n 'blocking::Client::builder' src/providers/mod.rs`
      2. For each match, verify proxy application within 10 lines
      3. Run `cargo test`
      4. Verify all tests pass
    Expected Result: All blocking client builders have proxy applied
    Failure Indicators: Bare Client::builder() without proxy
    Evidence: .sisyphus/evidence/task-4-oauth-proxy-fix.txt
  ```
  **Commit**: YES
  - Message: `fix(providers): add proxy to OAuth blocking clients`
  - Files: `src/providers/mod.rs`
  - Pre-commit: `cargo test`

- [x] 5. Domestic Bypass Module (Domain List + IP GeoLocation API Fallback)
  **What to do**:
  - Create `src/vpn/bypass.rs` with two层判断机制:
    - 第一层：内置域名列表快速匹配（零延迟）
    - 第二层：IP 归属地 API 兜底查询（域名不在列表中时）
  - 内置域名列表: `*.baidu.com`, `*.bilibili.com`, `*.feishu.cn`, `*.larksuite.com`, `*.dingtalk.com`, `*.qq.com`, `*.weixin.qq.com`, `*.wechat.com`, `*.taobao.com`, `*.tmall.com`, `*.alipay.com`, `*.jd.com`, `*.douyin.com`, `*.zhihu.com`, `*.weibo.com`, `*.163.com`, `*.126.com`, `*.bytedance.com`, `*.xiaohongshu.com`, `*.meituan.com`, `*.didi.com`, `*.ctrip.com`, `*.aliyun.com`, `*.tencent.com`, `*.huawei.com`, `*.cn`, `*.com.cn`
  - IP GeoLocation API 集成:
    - 使用 `GET https://uapis.cn/api/v1/network/ipinfo?ip={ip}` 查询 IP 归属地
    - 响应中 `country` 字段为 `中国`/`CN` 时判定为国内
    - 本地 LRU 缓存已查询的 IP 结果（避免重复 API 调用）
    - 缓存 TTL: 24 小时，缓存容量: 10000 条
    - API 查询超时: 3 秒，超时时默认走 VPN（安全侧）
    - API 查询本身不走 VPN（直连）
  - Struct `BypassChecker` with methods:
    - `fn check_domain(&self, domain: &str) -> BypassDecision` — 域名列表快速匹配
    - `async fn check_ip(&self, ip: &str) -> BypassDecision` — IP 归属地 API 查询（带缓存）
    - `async fn should_bypass(&self, host: &str) -> bool` — 综合判断：先查域名列表，miss 则 DNS 解析后查 IP API
    - `fn add_domain(&mut self, domain: String)` — agent 运行时添加域名
    - `fn remove_domain(&mut self, domain: &str)` — agent 运行时移除域名
    - `fn to_no_proxy_list(&self) -> Vec<String>` — 导出域名列表给 reqwest no_proxy
  - Enum `BypassDecision`: `Bypass`（国内直连）, `Proxy`（走 VPN）, `Unknown`（API 失败，默认走 VPN）
  - Domain matching: suffix-based (e.g. `*.baidu.com` matches `www.baidu.com`)
  - IP 缓存持久化到 `~/.zeroclaw/state/vpn/ip_cache.json`（可选，启动时加载）
  - TDD: Write tests for domain matching, IP API response parsing, cache hit/miss, timeout fallback
  **Must NOT do**:
  - Do not embed GeoIP database（使用在线 API）
  - Do not block on API failure（超时默认走 VPN）
  - API 查询本身不能走 VPN（会死循环）
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: HTTP API 集成 + LRU 缓存 + async DNS 解析 + 两层判断逻辑
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (after Tasks 1, 2)
  - **Parallel Group**: Wave 1 (with Tasks 1, 2, 3, 4)
  - **Blocks**: Tasks 10, 11
  - **Blocked By**: Tasks 1, 2
  **References**:
  **Pattern References**:
  - `src/security/mod.rs` — `DomainMatcher` if it exists — check for existing domain matching utilities
  - `src/config/schema.rs:1149-1151` — `no_proxy` field pattern (Vec<String> of domain patterns)
  - `src/memory/embeddings.rs` — LRU cache pattern if available
  **External References**:
  - IP 归属地 API: `https://uapis.cn/api/v1/network/ipinfo?ip={ip}` — 返回 JSON 含 `country` 字段
  - API 文档: `https://uapis.cn/docs/api-reference/get-network-ipinfo`
  **WHY Each Reference Matters**:
  - DomainMatcher: May already have suffix-based domain matching — reuse if available
  - no_proxy: Output format must be compatible with reqwest's NoProxy parsing
  - IP API: 兜底判断的核心依赖，需要理解响应格式
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn bypass` passes
  - [ ] Default list contains at least 20 domestic domains
  - [ ] `check_domain("www.baidu.com")` returns `Bypass`
  - [ ] `check_domain("google.com")` returns `Proxy`（不在列表中）
  - [ ] IP API 响应解析正确（mock 测试）
  - [ ] 缓存命中时不调用 API
  - [ ] API 超时返回 `Unknown`（默认走 VPN）
  - [ ] `merge()` adds user domains without removing defaults
  **QA Scenarios:**
  ```
  Scenario: Domain list fast path — known domestic domain
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn bypass_domain_list_baidu_is_bypass`
      2. Run `cargo test --features vpn bypass_domain_list_google_is_not_bypass`
      3. Verify both pass
    Expected Result: Domestic domains matched instantly, foreign domains fall through
    Failure Indicators: False positive or false negative
    Evidence: .sisyphus/evidence/task-5-domain-matching.txt
  Scenario: IP API fallback — cache miss then hit
    Tool: Bash
    Preconditions: Mock HTTP server or mock reqwest response
    Steps:
      1. Run `cargo test --features vpn bypass_ip_api_cache_miss_calls_api`
      2. Run `cargo test --features vpn bypass_ip_api_cache_hit_skips_api`
      3. Verify both pass
    Expected Result: First call hits API, second call uses cache
    Failure Indicators: Cache not populated or API called twice
    Evidence: .sisyphus/evidence/task-5-ip-cache.txt
  Scenario: API timeout defaults to VPN (safe side)
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn bypass_ip_api_timeout_returns_unknown`
      2. Verify test passes
    Expected Result: BypassDecision::Unknown (caller treats as Proxy)
    Failure Indicators: Hang or Bypass decision on timeout
    Evidence: .sisyphus/evidence/task-5-ip-timeout.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add domestic bypass with domain list + IP geo API fallback`
  - Files: `src/vpn/bypass.rs`, `src/vpn/mod.rs`
  - Pre-commit: `cargo test --features vpn`
- [x] 6. Clash Subscription Parser (subconverter-rs Integration)
  **What to do**:
  - Create `src/vpn/subscription.rs`
  - Use `subconverter` crate (lonelam/subconverter-rs) to parse Clash subscription URLs
  - Struct `SubscriptionParser` with methods:
    - `async fn fetch_and_parse(url: &str) -> Result<Vec<ProxyNode>>` — fetch subscription URL, parse YAML, return node list
    - `fn parse_clash_yaml(content: &str) -> Result<Vec<ProxyNode>>` — parse raw Clash YAML content
  - Struct `ProxyNode` with fields: `name: String`, `node_type: NodeType` (VMess/VLESS/Trojan/SS/HTTP/SOCKS5), `server: String`, `port: u16`, `raw_config: serde_json::Value` (full node config for clash-lib)
  - Enum `NodeType`: VMess, VLESS, Trojan, Shadowsocks, Http, Socks5, Other(String)
  - Handle subscription fetch errors gracefully (timeout, 403, invalid YAML)
  - Use `build_runtime_proxy_client("vpn.subscription")` for fetching (subscription fetch itself may need proxy)
  - TDD: Write tests with sample Clash YAML fixtures (embedded in test module)
  **Must NOT do**:
  - Do not implement proxy protocol handling (that's clash-lib's job)
  - Do not support non-Clash subscription formats yet
  - Do not store nodes (that's Task 8's job)
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: External crate integration, YAML parsing, error handling complexity
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (after Tasks 1, 2)
  - **Parallel Group**: Wave 2 (with Tasks 7, 8, 9)
  - **Blocks**: Tasks 8, 10
  - **Blocked By**: Tasks 1, 2
  **References**:
  **Pattern References**:
  - `src/memory/embeddings.rs:63` — `build_runtime_proxy_client("memory.embeddings")` pattern for HTTP client creation
  **External References**:
  - subconverter-rs: `https://github.com/lonelam/subconverter-rs` — API surface, Clash YAML parsing functions
  - Clash proxy config format: `https://dreamacro.github.io/clash/configuration/outbound.html` — YAML structure for proxies
  **WHY Each Reference Matters**:
  - embeddings.rs: Pattern for creating proxy-aware HTTP client for subscription fetch
  - subconverter-rs API: Need to understand which functions parse Clash YAML and what output format
  - Clash docs: Canonical YAML structure for proxy node definitions
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn subscription` passes
  - [ ] Sample Clash YAML with VMess/Trojan/SS nodes parses correctly
  - [ ] Invalid YAML returns descriptive error (not panic)
  - [ ] Empty proxy list returns Ok(vec![])
  **QA Scenarios:**
  ```
  Scenario: Parse sample Clash YAML with multiple node types
    Tool: Bash
    Preconditions: Test fixtures embedded in test module
    Steps:
      1. Run `cargo test --features vpn subscription_parses_vmess_node`
      2. Run `cargo test --features vpn subscription_parses_trojan_node`
      3. Run `cargo test --features vpn subscription_parses_ss_node`
      4. Verify all pass
    Expected Result: Each node type parsed with correct fields
    Failure Indicators: Parse error or wrong node type
    Evidence: .sisyphus/evidence/task-6-subscription-parse.txt
  Scenario: Invalid YAML returns error not panic
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn subscription_invalid_yaml_returns_error`
      2. Verify test passes
    Expected Result: Err variant with descriptive message
    Failure Indicators: Panic or Ok with empty list
    Evidence: .sisyphus/evidence/task-6-subscription-error.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add Clash subscription parser`
  - Files: `src/vpn/subscription.rs`, `src/vpn/mod.rs`
  - Pre-commit: `cargo test --features vpn`
- [x] 7. clash-lib Runtime Manager
  **What to do**:
  - Create `src/vpn/runtime.rs`
  - Use `clash-lib` crate to embed a local proxy runtime
  - Struct `ClashRuntime` with methods:
    - `async fn start(config: &str, listen_port: u16) -> Result<Self>` — generate Clash config YAML from node list, start clash-lib with `start_scaffold(Options { config: Config::Str(...), ... })`
    - `async fn stop(&self) -> Result<()>` — call `clash_lib::shutdown()`
    - `fn is_running(&self) -> bool`
    - `fn local_proxy_url(&self) -> String` — returns `socks5://127.0.0.1:{listen_port}`
    - `async fn switch_node(&self, node_name: &str) -> Result<()>` — update active node via clash-lib API
  - Generate minimal Clash config YAML: `socks-port`, `bind-address: 127.0.0.1`, `proxies` list, `proxy-groups` with selector
  - Ensure clash-lib uses the existing tokio runtime (pass `TokioRuntime` option)
  - Handle startup failures gracefully (port in use, invalid config)
  - TDD: Write tests for config generation, start/stop lifecycle (mock or integration)
  **Must NOT do**:
  - Do not expose clash-lib's HTTP API externally (bind 127.0.0.1 only)
  - Do not implement TUN mode
  - Do not allow configurable bind address (always localhost)
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: External crate integration, async lifecycle management, config generation
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (after Tasks 1, 2)
  - **Parallel Group**: Wave 2 (with Tasks 6, 8, 9)
  - **Blocks**: Tasks 9, 10, 11
  - **Blocked By**: Tasks 1, 2
  **References**:
  **External References**:
  - clash-rs GitHub: `https://github.com/Watfaq/clash-rs` — `clash-lib/src/lib.rs` for `start_scaffold`, `Options`, `Config::Str`, `shutdown` API
  - clash-lib Options struct: `config: Config`, `cwd: Option<String>`, `rt: Option<TokioRuntime>`, `log_file: Option<String>`
  **Pattern References**:
  - `src/tunnel/cloudflare.rs` or `src/tunnel/ngrok.rs` — Pattern for managing external process lifecycle (start/stop/health), though clash-lib is in-process
  **WHY Each Reference Matters**:
  - clash-lib API: Core integration point — must understand Options, Config enum, shutdown mechanism
  - tunnel modules: Lifecycle management pattern (start, stop, is_running) to follow
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn runtime` passes
  - [ ] Config generation produces valid Clash YAML
  - [ ] `local_proxy_url()` returns correct format
  - [ ] Start with invalid config returns error (not panic)
  **QA Scenarios:**
  ```
  Scenario: Generate valid Clash config YAML
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn runtime_generates_valid_clash_config`
      2. Verify test passes
    Expected Result: Generated YAML contains socks-port, bind-address, proxies
    Failure Indicators: YAML parse error or missing required fields
    Evidence: .sisyphus/evidence/task-7-config-generation.txt
  Scenario: Start with invalid config returns error
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn runtime_invalid_config_returns_error`
      2. Verify test passes
    Expected Result: Err variant, not panic
    Failure Indicators: Panic or unwrap failure
    Evidence: .sisyphus/evidence/task-7-invalid-config.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add clash-lib runtime manager`
  - Files: `src/vpn/runtime.rs`, `src/vpn/mod.rs`
  - Pre-commit: `cargo test --features vpn`
- [x] 8. Node Persistence (Disk Cache)
  **What to do**:
  - Create node persistence logic in `src/vpn/node_manager.rs` (partial — persistence only)
  - Struct `NodeCache` with methods:
    - `async fn save(nodes: &[ProxyNode], path: &Path) -> Result<()>` — serialize to JSON, write to disk
    - `async fn load(path: &Path) -> Result<Option<Vec<ProxyNode>>>` — read from disk, deserialize
    - `fn cache_path(config: &VpnConfig) -> PathBuf` — resolve `~/.zeroclaw/state/vpn/nodes.json`
  - Create state directory if not exists (`~/.zeroclaw/state/vpn/`)
  - Handle corrupt file gracefully (log warning, return None)
  - Store last-fetched timestamp alongside nodes
  - TDD: Write tests for save/load roundtrip, corrupt file handling, missing directory creation
  **Must NOT do**:
  - Do not implement node selection logic (that's Task 10)
  - Do not implement health check (that's Task 9)
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: File I/O with error handling, JSON serialization, directory management
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (after Tasks 1, 2, 6)
  - **Parallel Group**: Wave 2 (with Tasks 6, 7, 9)
  - **Blocks**: Task 10
  - **Blocked By**: Tasks 1, 2, 6 (needs ProxyNode type from Task 6)
  **References**:
  **Pattern References**:
  - `src/channels/whatsapp_web.rs` or `src/auth/` — Pattern for state file persistence in `~/.zeroclaw/state/`
  - `src/config/schema.rs` — `workspace_dir` / state directory resolution patterns
  **WHY Each Reference Matters**:
  - State persistence: Follow existing patterns for creating state directories and writing JSON files
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn node_cache` passes
  - [ ] Save then load roundtrip preserves all node data
  - [ ] Corrupt JSON file returns Ok(None) with warning log
  - [ ] Missing directory is created automatically
  **QA Scenarios:**
  ```
  Scenario: Save/load roundtrip preserves data
    Tool: Bash
    Preconditions: Temp directory for test
    Steps:
      1. Run `cargo test --features vpn node_cache_roundtrip`
      2. Verify test passes
    Expected Result: Loaded nodes match saved nodes exactly
    Failure Indicators: Data loss or corruption
    Evidence: .sisyphus/evidence/task-8-cache-roundtrip.txt
  Scenario: Corrupt file handled gracefully
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn node_cache_corrupt_file_returns_none`
      2. Verify test passes
    Expected Result: Ok(None) returned, no panic
    Failure Indicators: Panic or deserialization error propagated
    Evidence: .sisyphus/evidence/task-8-cache-corrupt.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add node persistence (disk cache)`
  - Files: `src/vpn/node_manager.rs`, `src/vpn/mod.rs`
  - Pre-commit: `cargo test --features vpn`
- [x] 9. Health Checker + Latency Tester
  **What to do**:
  - Create `src/vpn/health.rs`
  - Struct `HealthChecker` with methods:
    - `async fn check_node(node: &ProxyNode, proxy_url: &str) -> Result<HealthResult>` — TCP connect through proxy + HTTP probe to `http://connectivitycheck.gstatic.com/generate_204` (or configurable URL)
    - `async fn measure_latency(node: &ProxyNode, proxy_url: &str) -> Result<Duration>` — time the HTTP probe roundtrip
    - `async fn check_all(nodes: &[ProxyNode], proxy_url: &str) -> Vec<(String, HealthResult)>` — parallel health check all nodes
    - `fn start_background_loop(interval: Duration, callback: impl Fn(HealthResult))` — tokio::spawn periodic check
  - Struct `HealthResult`: `status: NodeStatus` (Healthy/Unhealthy/Unknown), `latency_ms: Option<u64>`, `checked_at: Instant`
  - Background loop: every `health_check_interval_secs` (default 30s), check current active node
  - If active node unhealthy → trigger callback (used by node_manager for failover)
  - Respect tokio CancellationToken for graceful shutdown
  - TDD: Write tests for health check logic, latency measurement, background loop start/stop
  **Must NOT do**:
  - Do not implement failover logic (that's Task 10)
  - Do not make health check URL configurable in this task (hardcode reasonable default)
  - Do not block on health check (always async with timeout)
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Async background loop, timeout handling, parallel checks, CancellationToken
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: YES (after Tasks 1, 2, 7)
  - **Parallel Group**: Wave 2 (with Tasks 6, 7, 8)
  - **Blocks**: Task 10
  - **Blocked By**: Tasks 1, 2, 7 (needs ClashRuntime proxy URL)
  **References**:
  **Pattern References**:
  - `src/heartbeat/mod.rs` — Background periodic loop pattern with tokio::spawn and interval
  - `src/health/mod.rs` — Health check patterns if they exist
  - `src/channels/traits.rs` — `health_check()` method pattern on Channel trait
  **External References**:
  - reqwest timeout: `reqwest::Client::builder().timeout(Duration::from_secs(5))` for probe timeout
  **WHY Each Reference Matters**:
  - heartbeat: Exact pattern for periodic background task with graceful shutdown
  - Channel health_check: Existing health check convention to follow
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn health` passes
  - [ ] Health check with unreachable host returns Unhealthy (not hang/panic)
  - [ ] Latency measurement returns Duration in reasonable range
  - [ ] Background loop respects CancellationToken
  **QA Scenarios:**
  ```
  Scenario: Unreachable node returns Unhealthy
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn health_unreachable_returns_unhealthy`
      2. Verify test passes within 10s (timeout works)
    Expected Result: HealthResult with status=Unhealthy
    Failure Indicators: Hang, panic, or Healthy status
    Evidence: .sisyphus/evidence/task-9-health-unreachable.txt
  Scenario: Background loop starts and stops cleanly
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn health_background_loop_shutdown`
      2. Verify test passes
    Expected Result: Loop starts, runs at least once, stops on cancel
    Failure Indicators: Hang on shutdown or panic
    Evidence: .sisyphus/evidence/task-9-health-loop.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add health checker and latency tester`
  - Files: `src/vpn/health.rs`, `src/vpn/mod.rs`
  - Pre-commit: `cargo test --features vpn`
- [ ] 10. Node Manager (Selection Strategy + Failover Logic)
  **What to do**:
  - Expand `src/vpn/node_manager.rs` (started in Task 8 with persistence)
  - Struct `NodeManager` with methods:
    - `async fn new(config: &VpnConfig) -> Result<Self>` — load cached nodes or fetch from subscription
    - `async fn refresh_nodes(&mut self) -> Result<()>` — re-fetch subscription, update cache
    - `fn select_best_node(&self, health_results: &[(String, HealthResult)]) -> Option<&ProxyNode>` — pick lowest latency healthy node
    - `async fn failover(&mut self) -> Result<Option<ProxyNode>>` — called when active node fails: select next best, or None if all fail
    - `fn active_node(&self) -> Option<&ProxyNode>`
    - `fn all_nodes(&self) -> &[ProxyNode]`
    - `async fn switch_to(&mut self, node_name: &str) -> Result<()>` — manual node switch
  - Selection strategy: sort by latency ascending, pick first healthy node
  - Failover: on active node failure, select next best healthy node. If ALL nodes unhealthy → return None (caller disables VPN)
  - Rate limit subscription refresh (min 5 min between refreshes)
  - TDD: Write tests for selection strategy, failover logic, refresh rate limiting
  **Must NOT do**:
  - Do not interact with ProxyConfig (that's Task 11)
  - Do not interact with clash-lib directly (use ClashRuntime from Task 7)
  - Do not implement the VPN tool (that's Task 12)
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Complex state management, selection algorithm, failover logic
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 3 (with Tasks 11, 12)
  - **Blocks**: Tasks 11, 12
  - **Blocked By**: Tasks 5, 6, 7, 8, 9
  **References**:
  **Pattern References**:
  - `src/providers/resilient.rs` — Resilient provider wrapper with fallback/retry logic (if exists)
  - `src/reliability/` or `src/config/schema.rs:100-102` — `ReliabilityConfig` for retry/fallback patterns
  **WHY Each Reference Matters**:
  - Resilient provider: Failover pattern (try primary, fall back to secondary) to adapt for node selection
  - ReliabilityConfig: Retry/backoff patterns to follow for subscription refresh rate limiting
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn node_manager` passes
  - [ ] Selects lowest latency healthy node
  - [ ] Failover skips unhealthy nodes
  - [ ] All nodes unhealthy → returns None
  - [ ] Refresh rate limited to min 5 min interval
  **QA Scenarios:**
  ```
  Scenario: Selects lowest latency node
    Tool: Bash
    Preconditions: Mock health results with varying latencies
    Steps:
      1. Run `cargo test --features vpn node_manager_selects_lowest_latency`
      2. Verify test passes
    Expected Result: Node with 50ms selected over 200ms and 500ms
    Failure Indicators: Wrong node selected
    Evidence: .sisyphus/evidence/task-10-selection.txt
  Scenario: All nodes unhealthy returns None
    Tool: Bash
    Preconditions: All health results Unhealthy
    Steps:
      1. Run `cargo test --features vpn node_manager_all_unhealthy_returns_none`
      2. Verify test passes
    Expected Result: failover() returns Ok(None)
    Failure Indicators: Panic or Some(node)
    Evidence: .sisyphus/evidence/task-10-all-unhealthy.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add node manager with selection strategy`
  - Files: `src/vpn/node_manager.rs`
  - Pre-commit: `cargo test --features vpn`
- [ ] 11. VPN ↔ ProxyConfig Bridge (Runtime Proxy Integration)
  **What to do**:
  - Create `src/vpn/bridge.rs`
  - Struct `VpnProxyBridge` with methods:
    - `fn activate(proxy_url: &str, bypass: &BypassList) -> Result<()>` — call `set_runtime_proxy_config()` with VPN proxy URL + bypass domains as no_proxy
    - `fn deactivate() -> Result<()>` — restore original ProxyConfig (saved before activation)
    - `fn is_active() -> bool`
    - `fn update_proxy_url(new_url: &str) -> Result<()>` — update proxy URL without full deactivate/activate cycle
  - On activate: save current `runtime_proxy_config()` as backup, then set new config with VPN proxy URL
  - On deactivate: restore saved backup config
  - Merge VPN no_proxy (from bypass list) with existing no_proxy entries
  - Use `ProxyScope::Services` or `ProxyScope::Zeroclaw` based on user config
  - Client cache automatically invalidated by `set_runtime_proxy_config()`
  - TDD: Write tests for activate/deactivate cycle, backup/restore, no_proxy merge
  **Must NOT do**:
  - Do not modify ProxyConfig struct — only use existing public API
  - Do not bypass `set_runtime_proxy_config()` — it's the single mutation path
  - Do not break existing `[proxy]` behavior when VPN is disabled
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: State management, backup/restore logic, integration with existing proxy system
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Task 10)
  - **Parallel Group**: Wave 3 (with Tasks 10, 12)
  - **Blocks**: Tasks 12, 13
  - **Blocked By**: Tasks 7, 10
  **References**:
  **Pattern References**:
  - `src/config/schema.rs:1491-1515` — `set_runtime_proxy_config()`, `runtime_proxy_config()` — the ONLY proxy state mutation API
  - `src/config/schema.rs:1134-1172` — `ProxyConfig` struct fields to understand what to set
  - `src/config/schema.rs:1234-1253` — `should_apply_to_service()` logic for scope-based routing
  - `src/tools/proxy_config.rs:249` — Pattern for saving config after proxy change
  **WHY Each Reference Matters**:
  - set_runtime_proxy_config: MUST use this as sole mutation path (clears client cache automatically)
  - ProxyConfig fields: Need to construct correct config with VPN proxy URL + no_proxy
  - should_apply_to_service: Understand how scope affects which services use proxy
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn bridge` passes
  - [ ] Activate sets proxy URL via `set_runtime_proxy_config()`
  - [ ] Deactivate restores original proxy config
  - [ ] Bypass domains merged into no_proxy list
  - [ ] Existing `[proxy]` config restored after VPN deactivation
  **QA Scenarios:**
  ```
  Scenario: Activate/deactivate preserves original proxy config
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn bridge_activate_deactivate_roundtrip`
      2. Verify test passes
    Expected Result: After deactivate, runtime_proxy_config() matches original
    Failure Indicators: Original config lost or corrupted
    Evidence: .sisyphus/evidence/task-11-bridge-roundtrip.txt
  Scenario: Bypass domains appear in no_proxy
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn bridge_bypass_in_no_proxy`
      2. Verify test passes
    Expected Result: no_proxy contains domestic domains after activate
    Failure Indicators: no_proxy empty or missing bypass domains
    Evidence: .sisyphus/evidence/task-11-bridge-bypass.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): bridge VPN to ProxyConfig runtime`
  - Files: `src/vpn/bridge.rs`, `src/vpn/mod.rs`
  - Pre-commit: `cargo test --features vpn`
- [ ] 12. vpn_control Tool Implementation
  **What to do**:
  - Create `src/tools/vpn_control.rs`
  - Implement `Tool` trait for `VpnControlTool`
  - Actions (via `action` parameter):
    - `status` — return VPN enabled/disabled, active node name, latency, health status, listen port
    - `enable` — start clash-lib runtime, activate proxy bridge, start health check loop
    - `disable` — stop health check, deactivate proxy bridge, stop clash-lib runtime
    - `list_nodes` — return all parsed nodes with health status and latency
    - `switch_node` — switch to named node (param: `node_name`)
    - `refresh` — re-fetch subscription, update node list, re-select best node
    - `add_bypass` — add domain to bypass list at runtime (param: `domain`)
    - `remove_bypass` — remove domain from bypass list (param: `domain`)
  - Security: require `can_act()` for enable/disable/switch/refresh, allow `status`/`list_nodes` in readonly
  - Return structured JSON for all actions
  - TDD: Write tests for each action's parameter validation and response format
  **Must NOT do**:
  - Do not implement VPN logic in the tool — delegate to NodeManager, ClashRuntime, VpnProxyBridge
  - Do not expose clash-lib internals in tool output
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Multi-action tool with security checks, delegates to multiple modules
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO (depends on Tasks 10, 11)
  - **Parallel Group**: Wave 3 (with Tasks 10, 11)
  - **Blocks**: Task 13
  - **Blocked By**: Tasks 10, 11
  **References**:
  **Pattern References**:
  - `src/tools/proxy_config.rs` — EXACT pattern to follow: multi-action tool with get/set/list, security checks, JSON responses
  - `src/tools/proxy_config.rs:41-59` — `require_write_access()` security check pattern
  - `src/tools/proxy_config.rs:129-169` — `handle_get()` and `handle_list_services()` response patterns
  - `src/tools/traits.rs` — `Tool` trait: `name()`, `description()`, `parameters_schema()`, `call()`
  **API/Type References**:
  - `src/tools/traits.rs:ToolResult` — `success: bool`, `output: String`, `error: Option<String>`
  **WHY Each Reference Matters**:
  - proxy_config.rs: Nearly identical tool structure — multi-action, security-gated, JSON responses. Copy this pattern exactly.
  - Tool trait: Must implement all required methods
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn vpn_control` passes
  - [ ] All 8 actions return valid JSON
  - [ ] `enable`/`disable`/`switch_node`/`refresh` blocked in readonly mode
  - [ ] `status`/`list_nodes` allowed in readonly mode
  - [ ] Invalid action returns descriptive error
  **QA Scenarios:**
  ```
  Scenario: Status action returns structured JSON
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn vpn_control_status_returns_json`
      2. Verify test passes
    Expected Result: JSON with enabled, active_node, latency_ms, health fields
    Failure Indicators: Parse error or missing fields
    Evidence: .sisyphus/evidence/task-12-status-json.txt
  Scenario: Write actions blocked in readonly
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo test --features vpn vpn_control_enable_blocked_readonly`
      2. Verify test passes
    Expected Result: ToolResult with success=false, error mentioning readonly
    Failure Indicators: Action succeeds in readonly mode
    Evidence: .sisyphus/evidence/task-12-readonly-block.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): add vpn_control tool`
  - Files: `src/tools/vpn_control.rs`
  - Pre-commit: `cargo test --features vpn`
- [ ] 13. Module Registration + Tool Wiring + Startup/Shutdown
  **What to do**:
  - Wire VPN module into ZeroClaw runtime:
    - Register `vpn_control` tool in `src/tools/mod.rs` behind `#[cfg(feature = "vpn")]`
    - Add VPN startup logic in daemon/agent entry points (if `vpn.enabled`): parse subscription → start clash-lib → activate bridge → start health loop
    - Add VPN shutdown logic: stop health loop → deactivate bridge → stop clash-lib
    - Add `"vpn.subscription"` to `SUPPORTED_PROXY_SERVICE_KEYS` in `src/config/schema.rs`
  - Startup sequence: load config → load cached nodes (or fetch) → select best node → start clash-lib → activate proxy bridge → start health checker
  - Shutdown sequence: cancel health checker → deactivate proxy bridge → shutdown clash-lib → save node cache
  - Handle startup failure gracefully (log error, continue without VPN)
  - TDD: Write tests for tool registration, startup/shutdown sequence
  **Must NOT do**:
  - Do not add `#[cfg(feature = "vpn")]` to more than 3 existing files (tools/mod.rs, lib.rs, and one entry point)
  - Do not make VPN startup failure block ZeroClaw startup
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
    - Reason: Cross-module wiring, lifecycle management, conditional compilation
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4 (with Task 14)
  - **Blocks**: Task 14
  - **Blocked By**: Tasks 2, 11, 12
  **References**:
  **Pattern References**:
  - `src/tools/mod.rs:191-331` — Tool registration pattern (`tool_arcs.push(Arc::new(...))`), look for conditional registration with `if config.browser.enabled`
  - `src/tools/mod.rs` — `all_tools_with_runtime()` function where tools are registered
  - `src/main.rs` or `src/daemon/` — Daemon startup sequence where subsystems are initialized
  - `src/config/schema.rs:16-45` — `SUPPORTED_PROXY_SERVICE_KEYS` for adding `"vpn.subscription"`
  **WHY Each Reference Matters**:
  - tools/mod.rs: Exact registration point for new tool, with conditional pattern to follow
  - Daemon startup: Where to add VPN initialization in the boot sequence
  - SUPPORTED_PROXY_SERVICE_KEYS: Must add vpn.subscription for proxy-aware subscription fetching
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn` passes
  - [ ] `cargo build --release` (no features) compiles without VPN code
  - [ ] `vpn_control` tool appears in tool list when `vpn` feature enabled
  - [ ] VPN startup failure logs error but doesn't crash ZeroClaw
  - [ ] `#[cfg(feature = "vpn")]` in at most 3 existing files
  **QA Scenarios:**
  ```
  Scenario: Feature flag isolation in build
    Tool: Bash
    Preconditions: Clean build
    Steps:
      1. Run `cargo build --release 2>&1 | grep -c 'clash\|subconverter'`
      2. Verify count is 0 (no VPN deps compiled)
      3. Run `cargo build --release --features vpn 2>&1 | grep -c 'clash\|subconverter'`
      4. Verify count > 0 (VPN deps compiled)
    Expected Result: VPN code only compiled with feature flag
    Failure Indicators: VPN deps in non-feature build
    Evidence: .sisyphus/evidence/task-13-feature-isolation.txt
  Scenario: Tool registration conditional
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `grep -n 'vpn_control\|VpnControlTool' src/tools/mod.rs`
      2. Verify matches are inside `#[cfg(feature = "vpn")]` block
    Expected Result: Tool only registered with vpn feature
    Failure Indicators: Unconditional registration
    Evidence: .sisyphus/evidence/task-13-tool-registration.txt
  ```
  **Commit**: YES
  - Message: `feat(vpn): wire VPN module into runtime`
  - Files: `src/vpn/mod.rs`, `src/tools/mod.rs`, `src/config/schema.rs`, entry point file
  - Pre-commit: `cargo test --features vpn && cargo check`
- [ ] 14. Integration Tests (Full Flow)
  **What to do**:
  - Create `tests/vpn_integration.rs` (behind `#[cfg(feature = "vpn")]`)
  - Test scenarios:
    - Config loading: config with `[vpn]` section loads correctly, config without `[vpn]` loads with defaults
    - Subscription parse → node cache → load roundtrip
    - Bypass list: domestic domains in no_proxy after VPN activation
    - Bridge activate/deactivate: runtime proxy config changes correctly
    - Node manager: selection with mock health data, failover sequence
    - Tool parameter validation: all vpn_control actions accept correct params
    - Feature flag: verify `cargo check` without `--features vpn` doesn't reference VPN types
  - Use test fixtures (embedded Clash YAML, mock health results)
  - No network calls in integration tests (mock HTTP responses)
  **Must NOT do**:
  - Do not require external services (no real Clash subscription URLs)
  - Do not start actual clash-lib runtime in tests (mock the runtime)
  - Do not test proxy_config tool (already has its own tests)
  **Recommended Agent Profile**:
  - **Category**: `deep`
    - Reason: Cross-module integration, mock setup, comprehensive scenario coverage
  - **Skills**: []
  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 4 (with Task 13)
  - **Blocks**: F1-F4
  - **Blocked By**: Tasks 3, 4, 13
  **References**:
  **Pattern References**:
  - `tests/` directory — Existing integration test patterns (if any exist)
  - `src/config/schema.rs` test module (bottom of file) — Config deserialization test patterns
  **WHY Each Reference Matters**:
  - Existing tests: Follow project conventions for test organization and assertions
  - Config tests: Pattern for testing config deserialization with/without optional sections
  **Acceptance Criteria**:
  - [ ] `cargo test --features vpn vpn_integration` passes
  - [ ] All test scenarios covered
  - [ ] No network calls in tests
  - [ ] `cargo test` (without vpn feature) still passes
  **QA Scenarios:**
  ```
  Scenario: Full integration test suite passes
    Tool: Bash
    Preconditions: All previous tasks completed
    Steps:
      1. Run `cargo test --features vpn` 
      2. Verify all tests pass (0 failures)
      3. Run `cargo test` (without vpn feature)
      4. Verify all tests pass (0 failures)
    Expected Result: All tests green with and without vpn feature
    Failure Indicators: Any test failure
    Evidence: .sisyphus/evidence/task-14-integration-tests.txt
  Scenario: Feature flag isolation verified
    Tool: Bash
    Preconditions: None
    Steps:
      1. Run `cargo check 2>&1`
      2. Verify no VPN-related compilation
      3. Run `cargo check --features vpn 2>&1`
      4. Verify VPN modules compile
    Expected Result: Clean compilation in both modes
    Failure Indicators: Compilation error in either mode
    Evidence: .sisyphus/evidence/task-14-feature-isolation.txt
  ```
  **Commit**: YES
  - Message: `test(vpn): add integration tests`
  - Files: `tests/vpn_integration.rs`
  - Pre-commit: `cargo test --features vpn && cargo test`
## Final Verification Wave

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read the plan end-to-end. For each "Must Have": verify implementation exists (read file, run command). For each "Must NOT Have": search codebase for forbidden patterns — reject with file:line if found. Check evidence files exist in .sisyphus/evidence/. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo clippy --all-targets --features vpn -- -D warnings` + `cargo fmt --all -- --check` + `cargo test --features vpn`. Review all changed files for: `as any`/`@ts-ignore` (N/A for Rust), `unwrap()` in non-test code, empty error handlers, `todo!()` macros, unused imports. Check AI slop: excessive comments, over-abstraction, generic names.
  Output: `Build [PASS/FAIL] | Clippy [PASS/FAIL] | Tests [N pass/N fail] | Files [N clean/N issues] | VERDICT`

- [ ] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Build with `--features vpn`. Verify: config without `[vpn]` loads, config with `[vpn]` loads, `vpn_control` tool responds to all actions, health check loop starts/stops, node list persists to disk, bypass list works. Save to `.sisyphus/evidence/final-qa/`.
  Output: `Scenarios [N/N pass] | Integration [N/N] | Edge Cases [N tested] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff. Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Detect cross-task contamination. Flag unaccounted changes. Verify `cargo build --release` (no features) produces no VPN code.
  Output: `Tasks [N/N compliant] | Contamination [CLEAN/N issues] | Unaccounted [CLEAN/N files] | VERDICT`

---

## Commit Strategy

| Task | Commit Message | Files | Pre-commit |
|------|---------------|-------|------------|
| 1 | `feat(vpn): add vpn feature flag and dependencies` | Cargo.toml | `cargo check --features vpn` |
| 2 | `feat(vpn): add VPN config schema and types` | src/config/schema.rs, src/vpn/mod.rs | `cargo test --features vpn` |
| 3 | `fix(tools): add proxy integration to web_search_tool` | src/tools/web_search_tool.rs | `cargo test` |
| 4 | `fix(providers): add proxy to OAuth blocking clients` | src/providers/mod.rs | `cargo test` |
| 5 | `feat(vpn): add domestic bypass with domain list + IP geo API fallback` | src/vpn/bypass.rs | `cargo test --features vpn` |
| 6 | `feat(vpn): add Clash subscription parser` | src/vpn/subscription.rs | `cargo test --features vpn` |
| 7 | `feat(vpn): add clash-lib runtime manager` | src/vpn/runtime.rs | `cargo test --features vpn` |
| 8 | `feat(vpn): add node persistence (disk cache)` | src/vpn/node_manager.rs (partial) | `cargo test --features vpn` |
| 9 | `feat(vpn): add health checker and latency tester` | src/vpn/health.rs | `cargo test --features vpn` |
| 10 | `feat(vpn): add node manager with selection strategy` | src/vpn/node_manager.rs | `cargo test --features vpn` |
| 11 | `feat(vpn): bridge VPN to ProxyConfig runtime` | src/vpn/bridge.rs | `cargo test --features vpn` |
| 12 | `feat(vpn): add vpn_control tool` | src/tools/vpn_control.rs | `cargo test --features vpn` |
| 13 | `feat(vpn): wire VPN module into runtime` | src/vpn/mod.rs, src/tools/mod.rs, src/main.rs | `cargo test --features vpn` |
| 14 | `test(vpn): add integration tests` | tests/vpn_integration.rs | `cargo test --features vpn` |

---

## Success Criteria

### Verification Commands
```bash
# Feature flag isolation
cargo build --release 2>&1 | grep -v vpn  # No VPN code compiled
cargo build --release --features vpn       # VPN code compiles

# Tests
cargo test --features vpn                  # All tests pass
cargo clippy --all-targets --features vpn -- -D warnings  # No warnings

# Config backward compat
cargo run --features vpn -- status         # Works with existing config.toml

# Binary size check
ls -lh target/release/zeroclaw             # Baseline without vpn
```

### Final Checklist
- [ ] All "Must Have" present
- [ ] All "Must NOT Have" absent
- [ ] All tests pass (with and without `--features vpn`)
- [ ] Clippy clean
- [ ] Config backward compatible
- [ ] Feature flag isolation verified
