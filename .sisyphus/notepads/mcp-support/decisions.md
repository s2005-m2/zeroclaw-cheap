# MCP Support — Decisions

## Architecture
 Independent crate: `crates/zeroclaw-mcp/`
 Config: `.mcp.json` only
 Tool naming: `mcp_{server}_{tool}`
 Concurrency: snapshot-and-swap
 MCP protocol version: 2024-11-05
