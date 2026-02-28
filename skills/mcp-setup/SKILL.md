# MCP Server Setup

Install and register MCP servers so their tools become available at runtime.

## Key Concept

`mcp_manage` is for **registering** an MCP server as a client connection — it does NOT install packages. You must install the server package first, then register it.

## Workflow

1. **Install** the MCP server package (pip, npm, etc.)
2. **Register** via `mcp_manage add` with the server's command and args
3. Tools appear immediately — no restart needed

## mcp_manage Usage

```
mcp_manage list                          # see running servers and tool counts
mcp_manage add name=X command=Y args=Z  # register and hot-load a server
mcp_manage remove name=X                # unregister a server
```

## Examples

### Python MCP server (pip)

```bash
# 1. Install
python -m venv ~/.zeroclaw/mcp-time
~/.zeroclaw/mcp-time/bin/pip install mcp-server-time
```

```
# 2. Register
mcp_manage add name="time" command="~/.zeroclaw/mcp-time/bin/python" args=["-m", "mcp_server_time"]
```

### Node MCP server (npx)

```
mcp_manage add name="filesystem" command="npx" args=["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
```

### Node MCP server (global install)

```bash
npm install -g @modelcontextprotocol/server-fetch
```

```
mcp_manage add name="fetch" command="mcp-server-fetch"
```

## Rules

- ALWAYS use `mcp_manage add` to register servers. NEVER manually edit config.toml or .mcp.json.
- ALWAYS install the package before calling `mcp_manage add`.
- After `mcp_manage add`, the server's tools are available immediately.
- Requires Full autonomy level for add/remove operations.
