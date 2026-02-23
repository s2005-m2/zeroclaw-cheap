# MCP Support — Learnings

## 2026-02-23 Session Start
 Plan: 14 tasks + 4 final verification tasks
 Strategy: TDD (RED → GREEN → REFACTOR)
 Wave 1 first: T1 (scaffolding), then T2+T3 in parallel
## MCP Crate Scaffolding - Learnings

**Date:** 2026-02-23

### Successful Patterns

1. **Crate structure follows robot-kit pattern:**
   - edition = "2021"
   - rust-version = "1.87"
   - authors = ["theonlyhennygod"]
   - license = "MIT" (for sub-crates)
   - repository = "https://github.com/zeroclaw-labs/zeroclaw"

2. **Workspace integration:**
   - Added to  array in root Cargo.toml line 2
   - Added as path dependency in root Cargo.toml [dependencies] section
   - Placement: after async-trait, before HMAC-SHA256 section

3. **Required dependencies for MCP:**
   - serde with derive feature
   - serde_json
   - tokio with rt-multi-thread, macros, time, sync, process, io-util
   - anyhow for error handling
   - async-trait for trait definitions
   - tracing for logging

4. **lib.rs minimal structure:**
   - Crate-level docstring explaining purpose
   - Commented module declarations for future implementation:
     - jsonrpc (JSON-RPC 2.0 protocol)
     - types (core data structures)
     - transport (stdio, SSE, WebSocket)
     - client (MCP client)
     - config (configuration)

### Verification Commands



Both pass successfully with zero errors.



## MCP Protocol Types Implementation (T14)

**Date:** 2026-02-23

### TDD Flow Success

- Tests written FIRST in both jsonrpc.rs and types.rs
- Implementation followed to make tests pass
- All 20 tests pass on first full run after fixing one type error

### Key Implementation Patterns

1. **JSON-RPC 2.0 types** (jsonrpc.rs):
   - RequestId uses #[serde(untagged)] enum for number/string flexibility
   - jsonrpc field defaults to "2.0" via default function
   - params/result/error use skip_serializing_if for Option types

2. **MCP types** (types.rs):
   - All structs use #[serde(rename_all = "camelCase")] for MCP spec compliance
   - Marker capabilities (ToolsCapability, ResourcesCapability, PromptsCapability) are empty structs with Default
   - McpContent uses content_type field with #[serde(rename = "type")] to avoid Rust keyword
   - McpToolCallResult.isError uses #[serde(rename = "isError")] for exact spec compliance

3. **Test coverage**: 20 tests total
   - jsonrpc.rs: 8 tests (RequestId, Request, Response success/error, Notification, Error)
   - types.rs: 12 tests (Implementation, InitializeParams/Result, ToolInfo/Params/Result, Content, Resource, Prompt, Message)

### Gotchas Fixed

- Doc comments on module declarations in lib.rs must use //! style or be placed before the item, not inline
- RequestId::String variant needs owned String, not &str (fixed in test_jsonrpc_response_error_roundtrip)

### Dependencies Used

- serde (derive feature): Debug, Clone, Serialize, Deserialize derives
- serde_json: json! macro and Value type
- No new dependencies added



## MCP Transport Layer Implementation (T2)

**Date:** 2026-02-23

### TDD Flow Success

- Tests written FIRST in transport.rs
- Implementation followed to make tests pass
- All 7 transport tests pass on first full run

### Key Implementation Patterns

1. **McpTransport trait** (async trait for transport abstraction):
   - `send()`: Send JSON-RPC request
   - `send_notification()`: Send JSON-RPC notification (no response)
   - `receive()`: Receive JSON-RPC response
   - `close()`: Close transport and cleanup resources

2. **StdioTransport implementation**:
   - Spawns child process via `tokio::process::Command`
   - Pipes stdin/stdout/stderr
   - Uses newline-delimited JSON framing (each message = one line)
   - `write_json()`: Serialize to JSON + write + newline + flush
   - `read_json()`: Read line from BufReader + deserialize
   - `close()`: Kill child process + wait for exit
   - `Drop` impl: Ensure cleanup on drop

3. **Constructor**:
   ```rust
   pub async fn new(
       command: &str,
       args: &[String],
       env: &HashMap<String, String>
   ) -> anyhow::Result<Self>
   ```

### Test Coverage (7 tests)

1. `test_stdio_transport_new_with_cat`: Process spawning (unix: cat, windows: PowerShell)
2. `test_stdio_transport_serialization`: Request serialization
3. `test_stdio_transport_notification_serialization`: Notification serialization
4. `test_stdio_transport_response_deserialization`: Success response deserialization
5. `test_stdio_transport_error_deserialization`: Error response deserialization
6. `test_stdio_transport_send_receive_echo`: Integration test with echo script (unix only)
7. `test_stdio_transport_close_kills_process`: Process cleanup verification
8. `test_newline_delimited_framing`: Framing logic verification

### Platform-Specific Code

- Used `#[cfg(unix)]` and `#[cfg(windows)]` for platform-specific tests
- Unix tests use `cat` and bash scripts
- Windows tests use PowerShell commands
- Core implementation is cross-platform (uses tokio::process::Command)

### Dependencies Used

- tokio: process, io (AsyncBufReadExt, AsyncWriteExt, BufReader)
- serde_json: Value type, to_string, from_str, from_value, to_value
- anyhow: Context trait for error handling
- async_trait: #[async_trait] macro
- tracing: debug!, info!, error! macros

### Gotchas Fixed

1. `Child::try_kill()` doesn't exist → use `kill().await` instead
2. `Child::start_try_wait()` doesn't exist → use `try_wait()` instead
3. `Child` doesn't implement Debug → add `#[derive(Debug)]` to StdioTransport
4. Error formatting in tests → removed `{:?}` format for Result types

### Module Registration

- Added `pub mod transport;` to lib.rs (between jsonrpc and types)
- Commented out future modules (client, config) remain
## MCP Client Resources and Prompts Implementation - 2026-02-23

### What was implemented:
- Added resource methods: list_resources(), read_resource()
- Added prompt methods: list_prompts(), get_prompt()
- Both methods follow the same JSON-RPC pattern as tool methods
- Error handling checks response.error field before parsing result

### Key patterns used:
1. Send JSON-RPC request with method name (resources/list, resources/read, prompts/list, prompts/get)
2. Receive response and check for error field
3. Parse result.{resources,contents,prompts,messages} array
4. Return typed Vec<McpResource>, Vec<McpResourceContent>, Vec<McpPrompt>, or Vec<McpPromptMessage>

### Verification:
- cargo check -p zeroclaw-mcp: PASSED
- cargo test -p zeroclaw-mcp: ALL 34 TESTS PASSED

### Notes:
- Module-level doc comment is necessary (priority 3) for public API documentation
- Removed Debug derive from McpClient because Box<dyn McpTransport> doesn't implement Debug
- Removed unused McpPromptArgument import

