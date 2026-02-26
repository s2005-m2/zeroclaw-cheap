//! Integration tests for the full hook lifecycle.
//!
//! These tests exercise the public API surface of the hooks subsystem:
//! HookRunner, DynamicHookHandler, LoadedHook, reload stamps, and
//! condition evaluation — wired together as they would be in production.

#[cfg(test)]
mod tests {

    use async_trait::async_trait;
    use serde_json::Value;
    use tempfile::TempDir;

    use crate::hooks::dynamic::DynamicHookHandler;
    use crate::hooks::loader::LoadedHook;
    use crate::hooks::manifest::{HookAction, HookConditions, HookEvent, HookManifest};
    use crate::hooks::reload::{check_reload_stamp, delete_reload_stamp, write_reload_stamp};
    use crate::hooks::runner::HookRunner;
    use crate::hooks::traits::{HookHandler, HookResult};

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    /// Build a minimal HookManifest for testing.
    fn make_manifest(name: &str, event: HookEvent, priority: i32) -> HookManifest {
        HookManifest {
            name: name.to_string(),
            description: None,
            version: None,
            event,
            priority,
            enabled: true,
            conditions: None,
            action: HookAction::Shell {
                command: "echo ok".to_string(),
                timeout_secs: Some(5),
                workdir: None,
            },
            skip_security_audit: false,
        }
    }

    /// Build a LoadedHook from a manifest and a temp directory.
    fn make_loaded_hook(manifest: HookManifest, dir: &TempDir) -> LoadedHook {
        LoadedHook {
            manifest,
            hook_dir: dir.path().to_path_buf(),
        }
    }

    /// Build a DynamicHookHandler from a LoadedHook.
    fn make_dynamic_handler(hook: LoadedHook) -> Box<dyn HookHandler> {
        Box::new(DynamicHookHandler::new(hook, 30))
    }

    /// A simple static hook for testing that records its name and priority.
    struct StaticTestHook {
        hook_name: String,
        hook_priority: i32,
    }

    impl StaticTestHook {
        fn new(name: &str, priority: i32) -> Self {
            Self {
                hook_name: name.to_string(),
                hook_priority: priority,
            }
        }
    }

    #[async_trait]
    impl HookHandler for StaticTestHook {
        fn name(&self) -> &str {
            &self.hook_name
        }
        fn priority(&self) -> i32 {
            self.hook_priority
        }
    }

    /// A static hook that cancels before_tool_call unconditionally.
    struct CancellingHook {
        hook_name: String,
        hook_priority: i32,
        reason: String,
    }

    impl CancellingHook {
        fn new(name: &str, priority: i32, reason: &str) -> Self {
            Self {
                hook_name: name.to_string(),
                hook_priority: priority,
                reason: reason.to_string(),
            }
        }
    }

    #[async_trait]
    impl HookHandler for CancellingHook {
        fn name(&self) -> &str {
            &self.hook_name
        }
        fn priority(&self) -> i32 {
            self.hook_priority
        }
        async fn before_tool_call(
            &self,
            _name: String,
            _args: Value,
        ) -> HookResult<(String, Value)> {
            HookResult::Cancel(self.reason.clone())
        }
    }

    // ---------------------------------------------------------------
    // Test 1: Full lifecycle — LoadedHook → DynamicHookHandler → HookRunner
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn full_lifecycle_dynamic_hook_fires_via_runner() {
        let dir = TempDir::new().unwrap();
        let manifest = make_manifest("lifecycle-hook", HookEvent::OnSessionStart, 5);
        let loaded = make_loaded_hook(manifest, &dir);
        let handler = make_dynamic_handler(loaded);

        let runner = HookRunner::new();
        // Load dynamic hooks via reload_dynamic_hooks (the production path)
        runner.reload_dynamic_hooks(vec![handler]).await;

        // Fire a void event — should not panic and should complete
        runner.fire_session_start("sess-001", "telegram").await;

        // Fire a modifying event the hook does NOT match — should pass through
        let result = runner
            .run_before_prompt_build("original prompt".to_string())
            .await;
        match result {
            HookResult::Continue(p) => assert_eq!(p, "original prompt"),
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }

    // ---------------------------------------------------------------
    // Test 2: Cancel propagation via before_tool_call
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn cancel_propagation_before_tool_call() {
        let mut runner = HookRunner::new();
        runner.register(Box::new(CancellingHook::new(
            "blocker",
            10,
            "tool blocked by policy",
        )));

        let result = runner
            .run_before_tool_call("shell".to_string(), serde_json::json!({"cmd": "rm -rf /"}))
            .await;

        match result {
            HookResult::Cancel(reason) => {
                assert_eq!(reason, "tool blocked by policy");
            }
            HookResult::Continue(_) => panic!("expected cancel, got continue"),
        }
    }

    // ---------------------------------------------------------------
    // Test 3: Priority ordering via before_prompt_build pipeline
    // ---------------------------------------------------------------

    /// A static hook that appends its name to the prompt, so we can verify order.
    struct AppendingHook {
        hook_name: String,
        hook_priority: i32,
    }

    impl AppendingHook {
        fn new(name: &str, priority: i32) -> Self {
            Self {
                hook_name: name.to_string(),
                hook_priority: priority,
            }
        }
    }

    #[async_trait]
    impl HookHandler for AppendingHook {
        fn name(&self) -> &str {
            &self.hook_name
        }
        fn priority(&self) -> i32 {
            self.hook_priority
        }
        async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
            HookResult::Continue(format!("{prompt}:{}", self.hook_name))
        }
    }

    #[tokio::test]
    async fn priority_ordering_before_prompt_build() {
        let mut runner = HookRunner::new();
        // Register in scrambled order; runner sorts by descending priority
        runner.register(Box::new(AppendingHook::new("mid", 5)));
        runner.register(Box::new(AppendingHook::new("low", 1)));
        runner.register(Box::new(AppendingHook::new("high", 10)));

        let result = runner.run_before_prompt_build("start".to_string()).await;
        match result {
            HookResult::Continue(p) => {
                // Highest priority fires first, then mid, then low
                assert_eq!(p, "start:high:mid:low");
            }
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }
    // ---------------------------------------------------------------
    // Test 4: Mixed static + dynamic hooks
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn mixed_static_and_dynamic_hooks() {
        let dir = TempDir::new().unwrap();
        let mut runner = HookRunner::new();

        // Register a static hook
        runner.register(Box::new(AppendingHook::new("static-hook", 20)));

        // Register a dynamic hook via reload
        let manifest = make_manifest("dynamic-hook", HookEvent::BeforePromptBuild, 5);
        let mut manifest_clone = manifest.clone();
        manifest_clone.action = HookAction::PromptInject {
            content: "[injected]".to_string(),
            position: Some("append".to_string()),
        };
        let loaded = make_loaded_hook(manifest_clone, &dir);
        let handler = make_dynamic_handler(loaded);
        runner.reload_dynamic_hooks(vec![handler]).await;

        // Both should fire: static (priority 20) first, then dynamic (priority 5)
        let result = runner.run_before_prompt_build("base".to_string()).await;
        match result {
            HookResult::Continue(p) => {
                // Static appends ":static-hook", then dynamic appends "\n[injected]"
                assert!(p.contains("static-hook"), "static hook should have fired: {p}");
                assert!(p.contains("[injected]"), "dynamic hook should have fired: {p}");
            }
            HookResult::Cancel(_) => panic!("should not cancel"),
        }

        // Reload dynamic hooks with empty set — static should still work
        runner.reload_dynamic_hooks(vec![]).await;
        let result2 = runner.run_before_prompt_build("base2".to_string()).await;
        match result2 {
            HookResult::Continue(p) => {
                assert_eq!(p, "base2:static-hook");
            }
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }
    // ---------------------------------------------------------------
    // Test 5: Stamp file lifecycle (write, check, check-again, delete)
    // ---------------------------------------------------------------
    #[test]
    fn stamp_file_full_lifecycle() {
        let dir = TempDir::new().unwrap();
        // No stamp yet — check returns false
        let mut last = None;
        assert!(!check_reload_stamp(dir.path(), &mut last));
        assert!(last.is_none());
        // Write stamp — first check returns true
        write_reload_stamp(dir.path()).unwrap();
        assert!(check_reload_stamp(dir.path(), &mut last));
        assert!(last.is_some());
        // Second check with same stamp — returns false (no change)
        assert!(!check_reload_stamp(dir.path(), &mut last));
        // Delete stamp
        delete_reload_stamp(dir.path()).unwrap();
        // After delete, check returns false
        assert!(!check_reload_stamp(dir.path(), &mut last));
        // Delete again (idempotent) — no error
        delete_reload_stamp(dir.path()).unwrap();
    }
    // ---------------------------------------------------------------
    // Test 6: Condition evaluation — channel condition match/skip
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn condition_evaluation_channel_match_and_skip() {
        let dir = TempDir::new().unwrap();
        // Create a hook with channel condition: only fires for "telegram"
        let mut manifest = make_manifest("cond-hook", HookEvent::OnMessageReceived, 5);
        manifest.conditions = Some(HookConditions {
            channels: Some(vec!["telegram".to_string()]),
            users: None,
            pattern: None,
        });
        manifest.action = HookAction::Shell {
            command: "echo matched".to_string(),
            timeout_secs: Some(5),
            workdir: None,
        };
        let loaded = make_loaded_hook(manifest, &dir);
        let handler = make_dynamic_handler(loaded);
        let runner = HookRunner::new();
        runner.reload_dynamic_hooks(vec![handler]).await;
        // Message from "telegram" channel — hook should fire (Continue)
        let msg_telegram = crate::channels::traits::ChannelMessage {
            id: "msg-1".to_string(),
            sender: "zeroclaw_user".to_string(),
            reply_target: "chat-1".to_string(),
            content: "hello".to_string(),
            channel: "telegram".to_string(),
            timestamp: 1000,
            thread_ts: None,
        };
        let result = runner.run_on_message_received(msg_telegram).await;
        assert!(!result.is_cancel(), "telegram message should not be cancelled");
        // Message from "discord" channel — hook condition doesn't match,
        // so hook is skipped and message passes through unchanged
        let msg_discord = crate::channels::traits::ChannelMessage {
            id: "msg-2".to_string(),
            sender: "zeroclaw_user".to_string(),
            reply_target: "chat-2".to_string(),
            content: "hello".to_string(),
            channel: "discord".to_string(),
            timestamp: 1001,
            thread_ts: None,
        };
        let result2 = runner.run_on_message_received(msg_discord).await;
        match result2 {
            HookResult::Continue(m) => {
                assert_eq!(m.channel, "discord");
                assert_eq!(m.content, "hello");
            }
            HookResult::Cancel(_) => panic!("discord message should pass through"),
        }
    }
}
