# Hook System — Decisions

## 2026-02-26 Architecture Decisions
- HOOK.toml manifest format (not Markdown)
- Stamp-file based hot-reload (no `notify` crate)
- RwLock for hook registry (no Mutex)
- DynamicHookHandler implements existing HookHandler trait (no trait changes)
- Security audit mirrors skills/audit.rs pattern
- Hook actions: shell, http, prompt_inject
