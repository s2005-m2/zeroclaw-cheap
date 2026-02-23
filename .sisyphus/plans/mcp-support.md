# ZeroClaw MCP Support — Full Implementation Plan

## TL;DR

> **Quick Summary**: Add full MCP client support to ZeroClaw as independent crate (`zeroclaw-mcp`), enabling AI to dynamically add/remove MCP servers at runtime. MCP tools injected directly into LLM's native tool list.
>
> **Deliverables**: `crates/zeroclaw-mcp/` crate, McpBridgeTool, McpManageTool, McpRegistry, .mcp.json loading, dynamic tool list rebuild
>
> **Estimated Effort**: Large
> **Parallel Execution**: YES — 4 waves
> **Critical Path**: T1 → T3 → T4 → T7 → T8 → T10 → T12 → T13

---

## Context

### Original Request
User wants ZeroClaw to support MCP like OpenClaw, where AI can dynamically add MCP servers at runtime without recompilation.

### Interview Summary
 **Approach**: Dynamic tool injection (MCP tools appear natively in LLM tool list)
 **Crate**: Independent `zeroclaw-mcp` in `crates/`
 **Config**: `.mcp.json` only (OpenClaw compatible)
 **Scope**: Full MCP (tools + resources + prompts)
 **AI self-add**: `mcp_manage` tool
 **Security**: MCP servers gated by SecurityPolicy

### Metis Review Gaps (addressed)
 Tool name collision → namespace `mcp_{server}_{tool}`
 Mutable tool list concurrency → snapshot-and-swap
 Windows subprocess → `tokio::process::Command`
 Tool count inflation → cap at 50 (configurable)
 `mcp_manage` security → `AutonomyLevel::Full` for arbitrary servers
 SSE transport → deferred (stdio covers 95%)
 `.mcp.json` hot-reload → use `mcp_manage` instead

---

## Work Objectives

### Core Objective
Enable ZeroClaw's AI agent to discover, connect to, and use MCP servers dynamically at runtime, with tools appearing natively in the LLM's tool list.

### Must Have
 MCP JSON-RPC 2.0 client with stdio transport
 tools/list, tools/call, resources/list, resources/read, prompts/list, prompts/get
 initialize/initialized handshake
 McpBridgeTool wrapping each MCP tool into ZeroClaw Tool trait
 mcp_manage tool (add/remove/list servers)
 .mcp.json config loading at startup
 Tool name namespacing: `mcp_{server}_{tool}`
 SecurityPolicy gating on MCP tool execution
 Independent crate with independent tests
 Cross-platform (Windows + Linux + macOS)
### Must NOT Have (Guardrails)
 MCP server mode (ZeroClaw as MCP server)
 MCP tools shadowing built-in tool names
 `mcp_manage` spawning arbitrary commands in Supervised mode
 SSE/HTTP transport in initial version
 File-watching for `.mcp.json` hot-reload
 MCP protocol dependencies in main `zeroclaw` crate
 Unbounded MCP tool count (cap at 50)

---

## Verification Strategy

> **ZERO HUMAN INTERVENTION** — ALL verification is agent-executed.

### Test Decision
 **Infrastructure exists**: YES (cargo test)
 **Automated tests**: YES (TDD — RED → GREEN → REFACTOR)
 **Framework**: cargo test (Rust built-in)
 **Crate tests**: `cargo test -p zeroclaw-mcp`
 **Full tests**: `cargo test` (workspace)
 **TDD Flow**: Each task writes failing test first (RED), then minimal implementation to pass (GREEN), then refactor
---
## Execution Strategy
### Waves
```
Wave 1 (Foundation): T1 scaffolding [quick], T2 protocol types [unspecified-high], T3 stdio transport [deep]
Wave 2 (Protocol): T4 client-tools [deep], T5 client-resources-prompts [unspecified-high], T6 .mcp.json parser [quick], T7 McpRegistry [deep]
Wave 3 (Integration): T8 McpBridgeTool [unspecified-high], T9 McpManageTool [unspecified-high], T10 mutable tool list [deep], T11 resource/prompt injection [unspecified-high]
Wave 4 (Wiring): T12 agent startup wiring [deep], T13 integration tests [deep], T14 docs [writing]
Wave FINAL: F1 compliance [oracle], F2 quality [unspecified-high], F3 QA [unspecified-high], F4 scope [deep]
```
### Dependency Matrix
| Task | Depends On | Blocks |
|------|-----------|--------|
| 1 | — | 2,3,6 |
| 2 | 1 | 4,5 |
| 3 | 1 | 4,5,7 |
| 4 | 2,3 | 7,8,13 |
| 5 | 2,3 | 11 |
| 6 | 1 | 7,9,12 |
| 7 | 3,4,6 | 8,9,10,12 |
| 8 | 4,7 | 10,12,13 |
| 9 | 7 | 12,13 |
| 10 | 7,8 | 12,13 |
| 11 | 5,7 | 12 |
| 12 | 8,9,10,11 | 13 |
| 13 | 12 | F1-F4 |
| 14 | 12 | F1 |
---
## TODOs

 [x] 1. Crate scaffolding + workspace wiring

  **What to do**:
  - Create `crates/zeroclaw-mcp/Cargo.toml` with deps: `serde`, `serde_json`, `tokio`, `anyhow`, `async-trait`, `tracing`
  - Create `crates/zeroclaw-mcp/src/lib.rs` with module declarations
  - Add `"crates/zeroclaw-mcp"` to workspace `members` in root `Cargo.toml`
  - Add `zeroclaw-mcp = { path = "crates/zeroclaw-mcp" }` to main crate dependencies
  - Verify `cargo check -p zeroclaw-mcp` passes

  **Must NOT do**: Add any MCP protocol dependencies to main zeroclaw crate

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: []

  **Parallelization**: Wave 1 | Blocks: 2,3,6 | Blocked By: None

  **References**:
  - `crates/robot-kit/Cargo.toml` — existing workspace crate structure to follow
  - `Cargo.toml:1-2` — workspace members array

  **Acceptance Criteria**:
  - [ ] `cargo check -p zeroclaw-mcp` exits 0
  - [ ] `cargo check` (workspace) exits 0

  **QA Scenarios**:
  ```
  Scenario: Crate compiles independently
    Tool: Bash
    Steps:
      1. Run `cargo check -p zeroclaw-mcp`
      2. Assert exit code 0
    Expected Result: Clean compilation with no errors
    Evidence: .sisyphus/evidence/task-1-crate-check.txt
  ```

  **Commit**: YES
  - Message: `feat(mcp): scaffold zeroclaw-mcp crate`
  - Files: `crates/zeroclaw-mcp/**`, `Cargo.toml`

 [x] 2. MCP protocol types
  **What to do (TDD)**:
  - RED: Write serde roundtrip tests for all types first (tests fail — types don't exist yet)
  - GREEN: Define JSON-RPC 2.0 types in `crates/zeroclaw-mcp/src/jsonrpc.rs`: `Request`, `Response`, `Error`, `Id`
  - GREEN: Define MCP message types in `crates/zeroclaw-mcp/src/types.rs`: `InitializeParams`, `InitializeResult`, `ServerCapabilities`, `McpToolInfo` (name, description, inputSchema), `McpToolCallParams`, `McpToolCallResult`, `McpResource`, `McpResourceContent`, `McpPrompt`, `McpPromptMessage`
  - GREEN: All types derive `Serialize`, `Deserialize`, `Debug`, `Clone` — tests pass
  - REFACTOR: Clean up type definitions if needed
  **Must NOT do**: Import anything from main zeroclaw crate
  **Recommended Agent Profile**: `unspecified-high`, Skills: []
  **Parallelization**: Wave 1 | Blocks: 4,5 | Blocked By: 1
  **References**:
  - MCP spec: `initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get` message schemas
  - JSON-RPC 2.0 spec: request/response/notification/error format
  **Acceptance Criteria**:
  - [ ] `cargo test -p zeroclaw-mcp` passes serde roundtrip tests
  - [ ] All MCP message types compile and serialize correctly
  **QA Scenarios**:
  ```
  Scenario: Protocol types serialize/deserialize correctly
    Tool: Bash
    Steps: Run `cargo test -p zeroclaw-mcp -- types`
    Expected Result: All type tests pass
    Evidence: .sisyphus/evidence/task-2-types-test.txt
  ```
  **Commit**: NO (groups with Wave 1)

 [x] 3. Stdio transport implementation
  **What to do (TDD)**:
  - RED: Write tests for McpTransport trait and StdioTransport first (tests fail)
  - GREEN: Define `McpTransport` trait in `crates/zeroclaw-mcp/src/transport.rs`: `async fn send(&mut self, msg: &Request)`, `async fn recv(&mut self) -> Response`
  - Implement `StdioTransport` using `tokio::process::Command` to spawn child process, read/write JSON-RPC over stdin/stdout with newline-delimited framing
  - GREEN: Handle process lifecycle: spawn, kill on drop, detect crash — tests pass
  - REFACTOR: Clean up if needed
  **Must NOT do**: Implement SSE/HTTP transport
  **Recommended Agent Profile**: `deep`, Skills: []
  **Parallelization**: Wave 1 | Blocks: 4,5,7 | Blocked By: 1
  **References**:
  - MCP spec stdio transport: newline-delimited JSON over stdin/stdout
  - `tokio::process::Command` for cross-platform subprocess
  **Acceptance Criteria**:
  - [ ] StdioTransport can spawn process and exchange JSON-RPC messages
  - [ ] Process cleanup on drop (no zombie processes)
  **QA Scenarios**:
  ```
  Scenario: Stdio transport sends and receives JSON-RPC
    Tool: Bash
    Steps: Run `cargo test -p zeroclaw-mcp -- transport`
    Expected Result: All transport tests pass
    Evidence: .sisyphus/evidence/task-3-transport-test.txt
  ```
  **Commit**: YES — `feat(mcp): add MCP protocol types and stdio transport`

 [x] 4. MCP client — tools (initialize + tools/list + tools/call)
  **What to do (TDD)**:
  - RED: Write tests for initialize handshake, list_tools, call_tool with mock transport (tests fail)
  - GREEN: Implement `McpClient` in `crates/zeroclaw-mcp/src/client.rs`
  - `async fn connect(transport) -> McpClient` — sends `initialize`, waits for result, sends `initialized` notification
  - `async fn list_tools() -> Vec<McpToolInfo>` — sends `tools/list`, parses response
  - `async fn call_tool(name, args) -> McpToolCallResult` — sends `tools/call`, returns content array
  - GREEN: Handle JSON-RPC error responses gracefully — tests pass
  - REFACTOR: Clean up client code if needed
  **Recommended Agent Profile**: `deep`, Skills: []
  **Parallelization**: Wave 2 | Blocks: 7,8,13 | Blocked By: 2,3
  **References**:
  - MCP spec: `initialize` params (protocolVersion, capabilities, clientInfo), `tools/list` response schema, `tools/call` params/response
  **Acceptance Criteria**:
  - [ ] McpClient completes initialize handshake with mock
  - [ ] `list_tools()` returns parsed McpToolInfo vec
  - [ ] `call_tool()` returns structured result
  **QA Scenarios**:
  ```
  Scenario: Client completes full tool lifecycle
    Tool: Bash
    Steps: Run `cargo test -p zeroclaw-mcp -- client::tools`
    Expected Result: All client tool tests pass
    Evidence: .sisyphus/evidence/task-4-client-tools.txt
  ```
  **Commit**: NO (groups with Wave 2)
 [x] 5. MCP client — resources + prompts
  **What to do**:
  - Add to `McpClient`: `list_resources()`, `read_resource(uri)`, `list_prompts()`, `get_prompt(name, args)`
  - Tests with mock transport
  **Recommended Agent Profile**: `unspecified-high`, Skills: []
  **Parallelization**: Wave 2 | Blocks: 11 | Blocked By: 2,3
  **Acceptance Criteria**:
  - [ ] All resource/prompt methods work with mock transport
  **QA Scenarios**:
  ```
  Scenario: Resources and prompts lifecycle
    Tool: Bash
    Steps: Run `cargo test -p zeroclaw-mcp -- client::resources client::prompts`
    Expected Result: All tests pass
    Evidence: .sisyphus/evidence/task-5-resources-prompts.txt
  ```
  **Commit**: YES — `feat(mcp): implement MCP client with full protocol support`
 [x] 6. .mcp.json config parser
  **What to do**:
  - Implement `parse_mcp_config(path) -> Vec<McpServerConfig>` in `crates/zeroclaw-mcp/src/config.rs`
  - `McpServerConfig`: `name`, `command`, `args`, `env`, `enabled`
  - Format: `{"mcpServers": {"name": {"command": "...", "args": [...], "env": {...}}}}`
  - Search order: workspace `.mcp.json` → `~/.zeroclaw/.mcp.json`
  - Tests for valid/invalid/missing config
  **Recommended Agent Profile**: `quick`, Skills: []
  **Parallelization**: Wave 2 | Blocks: 7,9,12 | Blocked By: 1
  **Acceptance Criteria**:
  - [ ] Parses valid `.mcp.json` into `Vec<McpServerConfig>`
  - [ ] Returns empty vec for missing file (not error)
  **QA Scenarios**:
  ```
  Scenario: Config parsing
    Tool: Bash
    Steps: Run `cargo test -p zeroclaw-mcp -- config`
    Expected Result: All config tests pass
    Evidence: .sisyphus/evidence/task-6-config.txt
  ```
  **Commit**: NO (groups with Wave 2)
- [ ] 7. McpRegistry — server connection manager
  **What to do**:
  - Create `src/mcp/registry.rs` with `McpRegistry` struct
  - `Arc<RwLock<HashMap<String, McpServerState>>>` where state = client + discovered tools
  - Methods: `add_server(name, config)`, `remove_server(name)`, `list_servers()`, `get_tools() -> Vec<McpToolInfo>`
  - Connect to server on add, disconnect on remove
  - Reject tool names colliding with built-in tools
  - Cap total MCP tools at configurable limit (default 50)
  **Recommended Agent Profile**: `deep`, Skills: []
  **Parallelization**: Wave 2 | Blocks: 8,9,10,12 | Blocked By: 3,4,6
  **References**:
  - `src/tools/composio.rs` — pattern for holding `RwLock` state in tool
  - `src/tools/mod.rs:196-316` — built-in tool names to check collisions against
  **Acceptance Criteria**:
  - [ ] Registry add/remove/list operations work
  - [ ] Built-in name collision rejected
  - [ ] Thread-safe concurrent access
  **QA Scenarios**:
  ```
  Scenario: Registry manages servers
    Tool: Bash
    Steps: Run `cargo test -- mcp::registry`
    Expected Result: All registry tests pass
    Evidence: .sisyphus/evidence/task-7-registry.txt
  ```
  **Commit**: YES — `feat(mcp): add McpRegistry server connection manager`
- [ ] 8. McpBridgeTool — Tool trait wrapper for MCP tools

  **What to do**:
  - Create `src/tools/mcp_bridge.rs` implementing `Tool` trait
  - Each `McpBridgeTool` wraps one MCP tool from a connected server
  - `name()` returns `mcp_{server}_{tool}` namespaced name
  - `description()` returns MCP tool's description
  - `parameters_schema()` returns MCP tool's inputSchema as serde_json::Value
  - `execute()` calls `McpRegistry::call_tool(server, tool, args)` and returns `ToolResult`
  - Convert MCP content array to ToolResult text (join text content items)
  - Handle MCP errors → ToolResult with error flag

  **Must NOT do**: Shadow built-in tool names (registry already rejects collisions)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**: Wave 3 | Blocks: 10,12,13 | Blocked By: 4,7

  **References**:
  - `src/tools/traits.rs` — `Tool` trait: `name()`, `description()`, `parameters_schema()`, `execute()`, `spec()`
  - `src/tools/composio.rs` — ComposioTool pattern for dynamic tool wrapping (holds Arc state, delegates execute)
  - `src/tools/mod.rs:196-316` — built-in tool registration pattern
  - `crates/zeroclaw-mcp/src/client.rs` — `McpClient::call_tool()` return type

  **Acceptance Criteria**:
  - [ ] McpBridgeTool implements Tool trait correctly
  - [ ] Namespaced name format: `mcp_{server}_{tool}`
  - [ ] MCP errors converted to ToolResult errors

  **QA Scenarios**:
  ```
  Scenario: Bridge tool delegates to MCP client
    Tool: Bash
    Steps:
      1. Run `cargo test -- tools::mcp_bridge`
      2. Assert all tests pass
    Expected Result: Bridge tool correctly wraps MCP tool calls
    Evidence: .sisyphus/evidence/task-8-bridge-tool.txt

  Scenario: Bridge tool handles MCP error gracefully
    Tool: Bash
    Steps:
      1. Run `cargo test -- tools::mcp_bridge::error`
      2. Assert error test passes
    Expected Result: MCP error → ToolResult with error=true and message
    Evidence: .sisyphus/evidence/task-8-bridge-error.txt
  ```

  **Commit**: NO (groups with Wave 3)

- [ ] 9. McpManageTool — AI self-management tool

  **What to do**:
  - Create `src/tools/mcp_manage.rs` implementing `Tool` trait
  - Tool name: `mcp_manage`
  - Parameters: `{"action": "add"|"remove"|"list", "name": "...", "command": "...", "args": [...], "env": {...}}`
  - `add`: validate config, call `McpRegistry::add_server()`, persist to `.mcp.json`
  - `remove`: call `McpRegistry::remove_server()`, remove from `.mcp.json`
  - `list`: return JSON array of connected servers with tool counts
  - Gate `add` action: require `AutonomyLevel::Full` for arbitrary server commands
  - In `Supervised` mode: only allow servers already in `.mcp.json` (pre-approved)
  - Register in `src/tools/mod.rs` alongside other tools

  **Must NOT do**: Allow `mcp_manage add` to spawn arbitrary commands in Supervised mode

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**: Wave 3 | Blocks: 12,13 | Blocked By: 7

  **References**:
  - `src/tools/traits.rs` — Tool trait
  - `src/security/mod.rs` — SecurityPolicy and AutonomyLevel checks
  - `src/config/schema.rs:AutonomyConfig` — autonomy level field
  - `src/tools/composio.rs` — pattern for tool holding Arc<RwLock> state

  **Acceptance Criteria**:
  - [ ] `mcp_manage list` returns server info
  - [ ] `mcp_manage add` creates connection and persists config
  - [ ] `mcp_manage add` blocked in Supervised mode for new servers
  - [ ] `mcp_manage remove` disconnects and removes config

  **QA Scenarios**:
  ```
  Scenario: AI adds MCP server via mcp_manage
    Tool: Bash
    Steps:
      1. Run `cargo test -- tools::mcp_manage::add`
      2. Assert test passes
    Expected Result: Server added, config persisted
    Evidence: .sisyphus/evidence/task-9-manage-add.txt

  Scenario: mcp_manage add blocked in Supervised mode
    Tool: Bash
    Steps:
      1. Run `cargo test -- tools::mcp_manage::supervised_block`
      2. Assert test passes
    Expected Result: Returns error when autonomy != Full
    Evidence: .sisyphus/evidence/task-9-manage-supervised.txt
  ```

  **Commit**: NO (groups with Wave 3)
- [ ] 10. Agent mutable tool list — snapshot-and-swap

  **What to do**:
  - Modify `src/agent/agent.rs`: change `tools: Vec<Box<dyn Tool>>` to `tools: Arc<RwLock<Vec<Box<dyn Tool>>>>`
  - Add `mcp_registry: Option<Arc<McpRegistry>>` field to `Agent`
  - In `AgentBuilder::build()`: if MCP config exists, create McpRegistry, connect to .mcp.json servers, generate McpBridgeTools
  - Modify `src/agent/loop_.rs`: at start of each `run_tool_call_loop` iteration, snapshot current tools from registry
  - Rebuild `tool_specs` from snapshot (built-in tools + current MCP bridge tools)
  - New MCP tools added via `mcp_manage` take effect on next loop iteration (snapshot-and-swap)
  **Must NOT do**: Lock tools during LLM call (snapshot before, release lock)
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**: Wave 3 | Blocks: 12,13 | Blocked By: 7,8
  **References**:
  - `src/agent/agent.rs:178` — `AgentBuilder::build()` where tools are constructed
  - `src/agent/loop_.rs:1904` — `run_tool_call_loop()` where tool_specs are rebuilt each iteration
  - `src/agent/agent.rs:Agent` struct — current `tools` and `tool_specs` fields
  **Acceptance Criteria**:
  - [ ] Agent accepts mutable tool list without breaking existing tool dispatch
  - [ ] Tool snapshot taken at loop start, not during LLM call
  - [ ] `cargo test` (workspace) passes — no regressions
  **QA Scenarios**:
  ```
  Scenario: Agent tool list updates between iterations
    Tool: Bash
    Steps:
      1. Run `cargo test -- agent::mcp_tool_refresh`
      2. Assert test passes
    Expected Result: Tools added to registry appear in next iteration's tool_specs
    Evidence: .sisyphus/evidence/task-10-mutable-tools.txt
  Scenario: Existing tools unaffected by MCP integration
    Tool: Bash
    Steps:
      1. Run `cargo test -- agent`
      2. Assert all existing agent tests still pass
    Expected Result: Zero regressions in agent test suite
    Evidence: .sisyphus/evidence/task-10-no-regression.txt
  ```
  **Commit**: YES — `feat(mcp): add mutable tool list with snapshot-and-swap`
- [ ] 11. MCP resource/prompt injection into agent context
  **What to do**:
  - In agent loop, after MCP registry snapshot, call `list_resources()` and `list_prompts()` on each connected server
  - Inject resource descriptions as additional system context (append to system prompt or context window)
  - Inject prompt templates as available prompt references the LLM can use
  - Format: `[MCP Resources] server_name: resource_uri — description` appended to system message
  - Format: `[MCP Prompts] server_name: prompt_name — description` appended to system message
  - Keep injection lightweight — list only, don't fetch full content until LLM requests via tool
  **Must NOT do**: Fetch all resource contents at startup (lazy load only)
  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
  **Parallelization**: Wave 3 | Blocks: 12 | Blocked By: 5,7
  **References**:
  - `src/agent/loop_.rs` — where system prompt is assembled before LLM call
  - `crates/zeroclaw-mcp/src/client.rs` — `list_resources()`, `list_prompts()` methods
  **Acceptance Criteria**:
  - [ ] MCP resources listed in agent system context
  - [ ] MCP prompts listed in agent system context
  - [ ] No full resource content fetched at startup
  **QA Scenarios**:
  ```
  Scenario: Resources and prompts appear in system context
    Tool: Bash
    Steps:
      1. Run `cargo test -- agent::mcp_context_injection`
      2. Assert test passes
    Expected Result: System prompt includes MCP resource/prompt listings
    Evidence: .sisyphus/evidence/task-11-context-injection.txt
  ```
  **Commit**: NO (groups with Wave 3)
- [ ] 12. Wire MCP into Agent startup + channel loop
  **What to do**:
  - In `src/agent/agent.rs` `AgentBuilder::build()`: load `.mcp.json`, create `McpRegistry`, connect to configured servers
  - Pass `Arc<McpRegistry>` to `McpManageTool` and `McpBridgeTool` constructors
  - Register `mcp_manage` tool in `src/tools/mod.rs` (conditional on MCP being configured)
  - In channel loop (`src/agent/loop_.rs`): integrate snapshot-and-swap for tool list refresh
  - Add `[mcp]` section to `src/config/schema.rs`: `enabled: bool`, `tool_cap: usize`, `config_path: Option<String>`
  - Log MCP server connections at startup: `tracing::info!("MCP: connected to {name}, {n} tools")`
  **Must NOT do**: Import MCP protocol types directly in agent code (use McpRegistry facade only)
  **Recommended Agent Profile**:
  - **Category**: `deep`
  - **Skills**: []
  **Parallelization**: Wave 4 | Blocks: 13 | Blocked By: 8,9,10,11
  **References**:
  - `src/agent/agent.rs:AgentBuilder` — where tools are assembled
  - `src/tools/mod.rs:all_tools_with_runtime()` — tool factory function
  - `src/config/schema.rs` — config struct definitions
  - `src/agent/loop_.rs:run_tool_call_loop()` — main agent loop
  **Acceptance Criteria**:
  - [ ] Agent starts with MCP servers from `.mcp.json` connected
  - [ ] MCP tools appear in LLM tool list
  - [ ] `mcp_manage` tool available to AI
  - [ ] `cargo test` passes — no regressions
  **QA Scenarios**:
  ```
  Scenario: Agent starts with MCP servers
    Tool: Bash
    Steps:
      1. Create test `.mcp.json` with a mock server config
      2. Run `cargo test -- agent::mcp_startup`
      3. Assert test passes
    Expected Result: Agent connects to configured MCP servers at startup
    Evidence: .sisyphus/evidence/task-12-startup.txt
  Scenario: Agent works without MCP config
    Tool: Bash
    Steps:
      1. Run `cargo test -- agent::no_mcp_config`
      2. Assert test passes
    Expected Result: Agent starts normally when no `.mcp.json` exists
    Evidence: .sisyphus/evidence/task-12-no-config.txt
  ```
  **Commit**: YES — `feat(mcp): wire MCP into agent startup and tool dispatch`
- [ ] 13. Integration tests
  **What to do**:
  - Create `tests/mcp_integration.rs` with end-to-end tests
  - Test: agent with `.mcp.json` → connects → MCP tools in tool_specs → call MCP tool → get result
  - Test: `mcp_manage add` → new tools appear next iteration
  - Test: `mcp_manage remove` → tools disappear next iteration
  - Test: tool name collision rejected at registry level
  - Test: tool cap enforced (51st tool rejected)
  - Use mock MCP server (simple Rust binary that speaks JSON-RPC over stdio)
  **Recommended Agent Profile**: `deep`, Skills: []
  **Parallelization**: Wave 4 | Blocks: F1-F4 | Blocked By: 12
  **References**:
  - All previous task outputs — this validates the full stack
  - `tests/` directory — existing integration test patterns
  **Acceptance Criteria**:
  - [ ] All integration tests pass
  - [ ] `cargo test --test mcp_integration` exits 0
  **QA Scenarios**:
  ```
  Scenario: Full MCP lifecycle integration
    Tool: Bash
    Steps:
      1. Run `cargo test --test mcp_integration`
      2. Assert all tests pass
    Expected Result: End-to-end MCP flow works
    Evidence: .sisyphus/evidence/task-13-integration.txt
  ```
  **Commit**: YES — `test(mcp): add MCP integration tests`
- [ ] 14. Documentation update
  **What to do**:
  - Add `[mcp]` section to `docs/config-reference.md` documenting all MCP config keys
  - Add MCP tool (`mcp_manage`) to `docs/commands-reference.md`
  - Add `.mcp.json` format documentation to `docs/config-reference.md`
  - Update `README.md` features table: add MCP row to subsystem table
  - Create `docs/mcp-guide.md` with: setup, .mcp.json format, mcp_manage usage, security notes
  **Recommended Agent Profile**: `writing`, Skills: []
  **Parallelization**: Wave 4 (parallel with T13) | Blocks: F1 | Blocked By: 12
  **References**:
  - `docs/config-reference.md` — existing config docs pattern
  - `docs/commands-reference.md` — existing command docs pattern
  - `README.md` — subsystem table to update
  **Acceptance Criteria**:
  - [ ] MCP config documented in config-reference.md
  - [ ] mcp_manage tool documented
  - [ ] .mcp.json format documented with examples
  **QA Scenarios**:
  ```
  Scenario: Docs contain MCP information
    Tool: Bash
    Steps:
      1. Run `grep -l 'mcp' docs/config-reference.md docs/commands-reference.md`
      2. Assert both files contain MCP references
    Expected Result: MCP documentation present in reference docs
    Evidence: .sisyphus/evidence/task-14-docs.txt
  ```
  **Commit**: YES — `docs(mcp): add MCP configuration and usage documentation`
---

## Final Verification Wave (after ALL implementation tasks)

> 4 review agents run in PARALLEL. ALL must APPROVE. Rejection → fix → re-run.

- [ ] F1. **Plan Compliance Audit** — `oracle`
  Read plan end-to-end. For each Must Have: verify implementation exists (read file, run command). For each Must NOT Have: search codebase for forbidden patterns. Check evidence files in `.sisyphus/evidence/`. Compare deliverables against plan.
  Output: `Must Have [N/N] | Must NOT Have [N/N] | Tasks [N/N] | VERDICT: APPROVE/REJECT`

- [ ] F2. **Code Quality Review** — `unspecified-high`
  Run `cargo fmt --all -- --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`. Review changed files for: `as any` casts, empty catches, `unwrap()` in non-test code, commented-out code, unused imports. Check AI slop: excessive comments, over-abstraction.
  Output: `Build [PASS/FAIL] | Lint [PASS/FAIL] | Tests [N pass/N fail] | VERDICT`
- [ ] F3. **Real Manual QA** — `unspecified-high`
  Start from clean state. Run `cargo test -p zeroclaw-mcp` and `cargo test` (workspace). Verify no MCP-related warnings in clippy. Check that agent starts cleanly with and without `.mcp.json`. Save all output to `.sisyphus/evidence/final-qa/`.
  Output: `Crate Tests [PASS/FAIL] | Workspace Tests [PASS/FAIL] | Clippy [CLEAN/N warnings] | VERDICT`

- [ ] F4. **Scope Fidelity Check** — `deep`
  For each task: read "What to do", read actual diff (`git log/diff`). Verify 1:1 — everything in spec was built, nothing beyond spec was built. Check "Must NOT do" compliance. Flag unaccounted changes.
  Output: `Tasks [N/N compliant] | Unaccounted [CLEAN/N files] | VERDICT`
---

## Commit Strategy

| Wave | Commit | Message | Pre-commit |
|------|--------|---------|------------|
| 1 | T1 | `feat(mcp): scaffold zeroclaw-mcp crate` | `cargo check` |
| 1 | T2+T3 | `feat(mcp): add MCP protocol types and stdio transport` | `cargo test -p zeroclaw-mcp` |
| 2 | T4+T5 | `feat(mcp): implement MCP client with full protocol support` | `cargo test -p zeroclaw-mcp` |
| 2 | T6+T7 | `feat(mcp): add McpRegistry server connection manager` | `cargo test` |
| 3 | T8+T9+T10+T11 | `feat(mcp): add mutable tool list with snapshot-and-swap` | `cargo test` |
| 4 | T12 | `feat(mcp): wire MCP into agent startup and tool dispatch` | `cargo test` |
| 4 | T13 | `test(mcp): add MCP integration tests` | `cargo test` |
| 4 | T14 | `docs(mcp): add MCP configuration and usage documentation` | — |
---

## Success Criteria

### Verification Commands
```bash
cargo check -p zeroclaw-mcp          # Crate compiles
cargo test -p zeroclaw-mcp           # Crate tests pass
cargo test                           # Workspace tests pass (no regressions)
cargo fmt --all -- --check           # Formatting clean
cargo clippy --all-targets -- -D warnings  # No warnings
cargo test --test mcp_integration    # Integration tests pass
```

### Final Checklist
- [ ] All 10 "Must Have" items present and verified
- [ ] All 7 "Must NOT Have" items absent from codebase
- [ ] `cargo test -p zeroclaw-mcp` passes
- [ ] `cargo test` (workspace) passes with zero regressions
- [ ] `cargo clippy` clean
- [ ] MCP tools appear in agent tool_specs when .mcp.json configured
- [ ] Agent works normally when no .mcp.json exists
- [ ] `mcp_manage add` blocked in Supervised mode
- [ ] Documentation updated (config-reference, commands-reference, mcp-guide)