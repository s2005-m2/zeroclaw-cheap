# Hook Management

Create, edit, and reload lifecycle hooks that run on agent events.

## Key Concept

Hooks are HOOK.toml manifests in `hooks/<name>/HOOK.toml`. You create/edit them with `file_write`, then call `hook_reload` to activate.

## Workflow

1. **Create** hook directory and HOOK.toml via `file_write`
2. **Reload** via `hook_reload` tool — runtime picks up changes
3. Hook fires on matching events automatically

## HOOK.toml Format

```toml
[hook]
name = "my-hook"
description = "Optional description"
event = "before_tool_call"
priority = 10
enabled = true

[hook.action.shell]
command = "echo hello"
timeout_secs = 30
workdir = "/tmp"
```

## Action Types

### Shell
```toml
[hook.action.shell]
command = "bash ./run.sh"
timeout_secs = 30
workdir = "/optional/path"
```

### HTTP
```toml
[hook.action.http]
url = "https://example.com/webhook"
method = "POST"
timeout_secs = 10
```

### Prompt Inject
```toml
[hook.action.prompt_inject]
content = "Always be helpful"
position = "prepend"
```

## Available Events

Void (fire-and-forget, parallel):
- `on_gateway_start`, `on_gateway_stop`
- `on_session_start`, `on_session_end`
- `on_llm_input`, `on_llm_output`
- `on_after_tool_call`
- `on_message_sent`
- `on_heartbeat_tick`

Modifying (sequential by priority, can cancel):
- `before_model_resolve`, `before_prompt_build`
- `before_llm_call`, `before_tool_call`
- `on_message_received`, `on_message_sending`
- `on_cron_delivery`, `on_docs_sync_notify`

## Optional Conditions

```toml
[hook.conditions]
channels = ["telegram", "discord"]
users = ["admin"]
pattern = ".*deploy.*"
```

## Examples

### Log every tool call
```toml
[hook]
name = "tool-logger"
event = "on_after_tool_call"

[hook.action.shell]
command = "echo \"[$(date -Iseconds)] tool called\" >> ~/tool_log.log"
```

### Inject system prompt
```toml
[hook]
name = "system-inject"
event = "before_prompt_build"

[hook.action.prompt_inject]
content = "You are a helpful assistant. Always respond in Chinese."
position = "prepend"
```

### Complex hook with script
1. Write script via `file_write` to `hooks/my-hook/run.sh`
2. Write HOOK.toml via `file_write` to `hooks/my-hook/HOOK.toml`:
```toml
[hook]
name = "my-hook"
event = "on_session_start"

[hook.action.shell]
command = "bash ./run.sh"
```
3. Call `hook_reload`

## Security

- Shell commands are audited for dangerous patterns (fork bombs, rm -rf /, reverse shells)
- Set `skip_security_audit = true` in HOOK.toml to bypass (not recommended)
- Hook name: alphanumeric + hyphens only, 1-64 chars

## Notes

- Higher priority number = runs first
- Disabled hooks (`enabled = false`) are skipped
- Modifying hooks can cancel the pipeline via `HookResult::Cancel`
- Hooks directory default: `{workspace}/hooks/`
