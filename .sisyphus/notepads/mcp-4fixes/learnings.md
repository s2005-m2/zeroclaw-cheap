
## MCP Context Accumulation Fix (agent.rs)

- The bug: every `turn()` call appended MCP resources/prompts to system message without removing old content
- Fix: added sentinel-based truncation (`\n[MCP Resources]\n` / `\n[MCP Prompts]\n`) before appending fresh context
- Key structural change: moved `if let Some(ConversationMessage::Chat(sys_msg))` to outer scope so truncation happens before conditional append
- The edit tool's auto-indentation can fight manual brace placement — need to replace ranges including surrounding context to get correct indent
- Pre-existing compile errors exist in: registry.rs (unclosed delimiters), wizard.rs, estop.rs, otp.rs, gateway/mod.rs, service/mod.rs — none related to agent.rs
- `rustfmt --check` returning only formatting diffs (not syntax errors) confirms valid syntax
- The em-dash character (\u{2014}) in format strings is preserved correctly via Unicode escape