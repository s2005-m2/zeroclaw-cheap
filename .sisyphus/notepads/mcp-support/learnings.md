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

