## Learnings

### Task 1: send_draft() card JSON streaming config
- `send_draft()` card JSON at ~line 1437 needed `streaming_mode: true`, `streaming_config`, and `element_id` on the markdown element
- Added `STREAMING_ELEMENT_ID` constant at line 19 (after existing constants block)
- The constant is referenced in the JSON via `serde_json::json!()` macro — Rust variables interpolate directly in the macro
- `cargo check` passes clean; `cargo test` hits pre-existing Windows PDB linker limit (LNK1318) unrelated to changes
- `create_card()` and `start_typing()` were correctly left untouched — only `send_draft()` needed the streaming card structure

### Task 5: Split message delivery flow in mod.rs
- `draft_updater` (spawned tokio task): added `card_closed` flag after `DRAFT_CLEAR_SENTINEL` clears accumulated text — prevents final LLM response from being pushed into the streaming card
- Success path: `finalize_draft("")` closes the card (empty string), then `send()` always delivers the full response via reply API — this is what triggers Lark's P2P reply
- All 3 error paths (context window, LLM error, timeout) follow the same pattern: `finalize_draft("")` then `send(error_text)`
- The `finalize_draft("")` with empty string is safe for non-Lark channels because the trait default is a no-op returning Ok(())
- Brace nesting in the LLM error path is tricky: the `if cancelled {} else if context_window {} else {}` chain is inside the match arm, so there are 3 levels of closing braces
- `cargo check` passes clean; only pre-existing warnings (unused variables/imports in cron/hooks modules)

### Task 6: Re-enable streaming, fix finalize_draft, add tests
- 6a: Replaced `StreamMode::Off` with `config.stream_mode.clone()` in all 3 `from_*config()` methods (lines 285, 301, 317)
- 6b: Replaced `finalize_draft()` body to call `close_streaming()` instead of the deleted `update_card()` method
  - Summary logic: empty text → "✅ Done", non-empty → first 20 chars
  - In practice text is always `""` from mod.rs (Task 5), so summary will be "✅ Done"
- 6c: Added 3 unit tests: `test_from_feishu_config_stream_mode`, `test_reply_message_url`, `test_parse_event_payload_p2p_thread_ts`
- Remaining `StreamMode::Off` matches in lark.rs (lines 1552, 1556) are runtime logic in `supports_draft_updates()` / `send_draft()` — correct and untouched
- `cargo check` passes with zero errors; only pre-existing warnings in cron/hooks modules
