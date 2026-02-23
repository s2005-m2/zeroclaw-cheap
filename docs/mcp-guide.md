# MCP (Model Context Protocol) Guide

Model Context Protocol (MCP) lets ZeroClaw connect to external tool servers. MCP servers expose tools, resources, and prompts that ZeroClaw can use natively.

## Overview

ZeroClaw MCP support:

- **stdio transport** — covers 95% of MCP servers (local process-based)
- **Native tool exposure** — MCP tools appear as `mcp_{server}_{tool}` in the LLM tool list
- **Self-management** — AI can add/remove MCP servers via the `mcp_manage` tool
- **Resource/prompt injection** — server resources and prompts are injected into system context

## Quick Start

1. **Enable MCP in config.toml:**

```toml
[mcp]
enabled = true
tool_cap = 50
config_path = ".mcp.json"
```

2. **Create `.mcp.json` in your workspace:**

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
    }
  }
}
```

3. **Restart ZeroClaw** — MCP tools appear automatically.

## `.mcp.json` Format

```json
{
  "mcpServers": {
    "server-name": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/allowed/path"],
      "env": {
        "SOME_VAR": "value"
      }
    }
  }
}
```

| Key | Required | Purpose |
|-----|----------|---------|
| `command` | Yes | Executable to run (e.g. `npx`, `node`, `python`) |
| `args` | Yes | Command arguments |
| `env` | No | Environment variables for the server process |

## mcp_manage Tool

The `mcp_manage` tool lets the AI manage MCP servers at runtime.

### Actions

**add** — Add a new MCP server (requires `AutonomyLevel::Full`):

```json
{
  "action": "add",
  "name": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
  "env": {}
}
```

**remove** — Remove an MCP server:

```json
{
  "action": "remove",
  "name": "filesystem"
}
```

**list** — List all connected MCP servers:

```json
{
  "action": "list"
}
```

### Security Notes

- **add requires Full autonomy** — adding MCP servers is a medium-risk operation
- MCP servers run as local processes with the same user permissions as ZeroClaw
- Keep `allowed_roots` and `forbidden_paths` configured to limit filesystem access
- Review MCP server code before adding — they execute with your user's privileges

## Configuration Reference

```toml
[mcp]
enabled = false           # Enable MCP support (default: false)
tool_cap = 50             # Max MCP tools across all servers (default: 50)
config_path = ".mcp.json" # Path to MCP config file (default: ".mcp.json")
```

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `enabled` | bool | `false` | Enable MCP tool loading |
| `tool_cap` | usize | `50` | Maximum number of MCP tools |
| `config_path` | `Option<String>` | `".mcp.json"` | Path to `.mcp.json` config |

## Related Docs

- [config-reference.md](config-reference.md) — full config reference
- [commands-reference.md](commands-reference.md) — CLI commands
