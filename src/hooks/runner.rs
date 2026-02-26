use std::sync::Arc;
use std::time::Duration;

use futures_util::{future::join_all, FutureExt};
use serde_json::Value;
use std::panic::AssertUnwindSafe;
use tokio::sync::RwLock;
use tracing::info;

use crate::channels::traits::ChannelMessage;
use crate::providers::traits::{ChatMessage, ChatResponse};
use crate::tools::traits::ToolResult;

use super::traits::{HookHandler, HookResult};

/// Dispatcher that manages registered hook handlers.
///
/// Void hooks are dispatched in parallel via `join_all`.
/// Modifying hooks run sequentially by priority (higher first), piping output
/// and short-circuiting on `Cancel`.
///
/// Static handlers are registered at compile-time and never swapped.
/// Dynamic handlers are loaded from HOOK.toml manifests and can be hot-reloaded.
pub struct HookRunner {
    static_handlers: Vec<Box<dyn HookHandler>>,
    dynamic_handlers: Arc<RwLock<Vec<Box<dyn HookHandler>>>>,
}

impl HookRunner {
    /// Create an empty runner with no handlers.
    pub fn new() -> Self {
        Self {
            static_handlers: Vec::new(),
            dynamic_handlers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a static (compile-time) handler and re-sort by descending priority.
    pub fn register(&mut self, handler: Box<dyn HookHandler>) {
        self.static_handlers.push(handler);
        self.static_handlers
            .sort_by_key(|h| std::cmp::Reverse(h.priority()));
    }

    /// Atomically replace all dynamic handlers with a new set.
    /// The new handlers are sorted by descending priority before storing.
    pub async fn reload_dynamic_hooks(&self, mut hooks: Vec<Box<dyn HookHandler>>) {
        hooks.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        let mut dynamic = self.dynamic_handlers.write().await;
        *dynamic = hooks;
    }

    // ---------------------------------------------------------------
    // Void dispatchers (parallel, fire-and-forget)
    // ---------------------------------------------------------------

    pub async fn fire_gateway_start(&self, host: &str, port: u16) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_gateway_start(host, port))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_gateway_stop(&self) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_gateway_stop())
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_session_start(&self, session_id: &str, channel: &str) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_session_start(session_id, channel))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_session_end(&self, session_id: &str, channel: &str) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_session_end(session_id, channel))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_llm_input(&self, messages: &[ChatMessage], model: &str) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_llm_input(messages, model))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_llm_output(&self, response: &ChatResponse) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_llm_output(response))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_after_tool_call(&self, tool: &str, result: &ToolResult, duration: Duration) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_after_tool_call(tool, result, duration))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_message_sent(&self, channel: &str, recipient: &str, content: &str) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_message_sent(channel, recipient, content))
            .collect();
        join_all(futs).await;
    }

    pub async fn fire_heartbeat_tick(&self) {
        let dynamic = self.dynamic_handlers.read().await;
        let futs: Vec<_> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.on_heartbeat_tick())
            .collect();
        join_all(futs).await;
    }

    // ---------------------------------------------------------------
    // Modifying dispatchers (sequential by priority, short-circuit on Cancel)
    // ---------------------------------------------------------------

    pub async fn run_before_model_resolve(
        &self,
        mut provider: String,
        mut model: String,
    ) -> HookResult<(String, String)> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.before_model_resolve(provider.clone(), model.clone()))
                .catch_unwind()
                .await
            {
                Ok(HookResult::Continue((p, m))) => {
                    provider = p;
                    model = m;
                }
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "before_model_resolve cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "before_model_resolve hook panicked; continuing with previous values"
                    );
                }
            }
        }
        HookResult::Continue((provider, model))
    }

    pub async fn run_before_prompt_build(&self, mut prompt: String) -> HookResult<String> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.before_prompt_build(prompt.clone()))
                .catch_unwind()
                .await
            {
                Ok(HookResult::Continue(p)) => prompt = p,
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "before_prompt_build cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "before_prompt_build hook panicked; continuing with previous value"
                    );
                }
            }
        }
        HookResult::Continue(prompt)
    }

    pub async fn run_before_llm_call(
        &self,
        mut messages: Vec<ChatMessage>,
        mut model: String,
    ) -> HookResult<(Vec<ChatMessage>, String)> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.before_llm_call(messages.clone(), model.clone()))
                .catch_unwind()
                .await
            {
                Ok(HookResult::Continue((m, mdl))) => {
                    messages = m;
                    model = mdl;
                }
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "before_llm_call cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "before_llm_call hook panicked; continuing with previous values"
                    );
                }
            }
        }
        HookResult::Continue((messages, model))
    }

    pub async fn run_before_tool_call(
        &self,
        mut name: String,
        mut args: Value,
    ) -> HookResult<(String, Value)> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.before_tool_call(name.clone(), args.clone()))
                .catch_unwind()
                .await
            {
                Ok(HookResult::Continue((n, a))) => {
                    name = n;
                    args = a;
                }
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "before_tool_call cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "before_tool_call hook panicked; continuing with previous values"
                    );
                }
            }
        }
        HookResult::Continue((name, args))
    }

    pub async fn run_on_message_received(
        &self,
        mut message: ChannelMessage,
    ) -> HookResult<ChannelMessage> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.on_message_received(message.clone()))
                .catch_unwind()
                .await
            {
                Ok(HookResult::Continue(m)) => message = m,
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "on_message_received cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "on_message_received hook panicked; continuing with previous message"
                    );
                }
            }
        }
        HookResult::Continue(message)
    }

    pub async fn run_on_message_sending(
        &self,
        mut channel: String,
        mut recipient: String,
        mut content: String,
    ) -> HookResult<(String, String, String)> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.on_message_sending(
                channel.clone(),
                recipient.clone(),
                content.clone(),
            ))
            .catch_unwind()
            .await
            {
                Ok(HookResult::Continue((c, r, ct))) => {
                    channel = c;
                    recipient = r;
                    content = ct;
                }
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "on_message_sending cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "on_message_sending hook panicked; continuing with previous message"
                    );
                }
            }
        }
        HookResult::Continue((channel, recipient, content))
    }

    pub async fn run_on_cron_delivery(
        &self,
        mut source: String,
        mut channel: String,
        mut recipient: String,
        mut content: String,
    ) -> HookResult<(String, String, String, String)> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.on_cron_delivery(
                source.clone(),
                channel.clone(),
                recipient.clone(),
                content.clone(),
            ))
            .catch_unwind()
            .await
            {
                Ok(HookResult::Continue((s, c, r, ct))) => {
                    source = s;
                    channel = c;
                    recipient = r;
                    content = ct;
                }
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "on_cron_delivery cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "on_cron_delivery hook panicked; continuing with previous values"
                    );
                }
            }
        }
        HookResult::Continue((source, channel, recipient, content))
    }
    pub async fn run_on_docs_sync_notify(
        &self,
        mut file_path: String,
        mut channel: String,
        mut recipient: String,
        mut content: String,
    ) -> HookResult<(String, String, String, String)> {
        let dynamic = self.dynamic_handlers.read().await;
        let mut all: Vec<&dyn HookHandler> = self
            .static_handlers
            .iter()
            .chain(dynamic.iter())
            .map(|h| h.as_ref())
            .collect();
        all.sort_by_key(|h| std::cmp::Reverse(h.priority()));
        for h in &all {
            let hook_name = h.name();
            match AssertUnwindSafe(h.on_docs_sync_notify(
                file_path.clone(),
                channel.clone(),
                recipient.clone(),
                content.clone(),
            ))
            .catch_unwind()
            .await
            {
                Ok(HookResult::Continue((fp, c, r, ct))) => {
                    file_path = fp;
                    channel = c;
                    recipient = r;
                    content = ct;
                }
                Ok(HookResult::Cancel(reason)) => {
                    info!(
                        hook = hook_name,
                        reason, "on_docs_sync_notify cancelled by hook"
                    );
                    return HookResult::Cancel(reason);
                }
                Err(_) => {
                    tracing::error!(
                        hook = hook_name,
                        "on_docs_sync_notify hook panicked; continuing with previous values"
                    );
                }
            }
        }
        HookResult::Continue((file_path, channel, recipient, content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// A hook that records how many times void events fire.
    struct CountingHook {
        name: String,
        priority: i32,
        fire_count: Arc<AtomicU32>,
    }

    impl CountingHook {
        fn new(name: &str, priority: i32) -> (Self, Arc<AtomicU32>) {
            let count = Arc::new(AtomicU32::new(0));
            (
                Self {
                    name: name.to_string(),
                    priority,
                    fire_count: count.clone(),
                },
                count,
            )
        }
    }

    #[async_trait]
    impl HookHandler for CountingHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        async fn on_heartbeat_tick(&self) {
            self.fire_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A modifying hook that uppercases the prompt.
    struct UppercasePromptHook {
        name: String,
        priority: i32,
    }

    #[async_trait]
    impl HookHandler for UppercasePromptHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
            HookResult::Continue(prompt.to_uppercase())
        }
    }

    /// A modifying hook that cancels before_prompt_build.
    struct CancelPromptHook {
        name: String,
        priority: i32,
    }

    #[async_trait]
    impl HookHandler for CancelPromptHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        async fn before_prompt_build(&self, _prompt: String) -> HookResult<String> {
            HookResult::Cancel("blocked by policy".into())
        }
    }

    /// A modifying hook that appends a suffix to the prompt.
    struct SuffixPromptHook {
        name: String,
        priority: i32,
        suffix: String,
    }

    #[async_trait]
    impl HookHandler for SuffixPromptHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
            HookResult::Continue(format!("{}{}", prompt, self.suffix))
        }
    }

    #[test]
    fn register_and_sort_by_priority() {
        let mut runner = HookRunner::new();
        let (low, _) = CountingHook::new("low", 1);
        let (high, _) = CountingHook::new("high", 10);
        let (mid, _) = CountingHook::new("mid", 5);

        runner.register(Box::new(low));
        runner.register(Box::new(high));
        runner.register(Box::new(mid));

        let names: Vec<&str> = runner.static_handlers.iter().map(|h| h.name()).collect();
        assert_eq!(names, vec!["high", "mid", "low"]);
    }

    #[tokio::test]
    async fn void_hooks_fire_all_handlers() {
        let mut runner = HookRunner::new();
        let (h1, c1) = CountingHook::new("hook_a", 0);
        let (h2, c2) = CountingHook::new("hook_b", 0);

        runner.register(Box::new(h1));
        runner.register(Box::new(h2));

        runner.fire_heartbeat_tick().await;

        assert_eq!(c1.load(Ordering::SeqCst), 1);
        assert_eq!(c2.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn modifying_hook_can_cancel() {
        let mut runner = HookRunner::new();
        runner.register(Box::new(CancelPromptHook {
            name: "blocker".into(),
            priority: 10,
        }));
        runner.register(Box::new(UppercasePromptHook {
            name: "upper".into(),
            priority: 0,
        }));

        let result = runner.run_before_prompt_build("hello".into()).await;
        assert!(result.is_cancel());
    }

    #[tokio::test]
    async fn modifying_hook_pipelines_data() {
        let mut runner = HookRunner::new();

        // Priority 10 runs first: uppercases
        runner.register(Box::new(UppercasePromptHook {
            name: "upper".into(),
            priority: 10,
        }));
        // Priority 0 runs second: appends suffix
        runner.register(Box::new(SuffixPromptHook {
            name: "suffix".into(),
            priority: 0,
            suffix: "_done".into(),
        }));

        match runner.run_before_prompt_build("hello".into()).await {
            HookResult::Continue(result) => assert_eq!(result, "HELLO_done"),
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }

    #[tokio::test]
    async fn reload_swaps_dynamic_handlers() {
        let mut runner = HookRunner::new();

        // Register a static hook
        let (static_hook, static_count) = CountingHook::new("static_hook", 0);
        runner.register(Box::new(static_hook));

        // Load initial dynamic hooks
        let (dyn_old, dyn_old_count) = CountingHook::new("dyn_old", 0);
        runner
            .reload_dynamic_hooks(vec![Box::new(dyn_old)])
            .await;

        // Fire once — both should fire
        runner.fire_heartbeat_tick().await;
        assert_eq!(static_count.load(Ordering::SeqCst), 1);
        assert_eq!(dyn_old_count.load(Ordering::SeqCst), 1);

        // Reload with a new dynamic hook
        let (dyn_new, dyn_new_count) = CountingHook::new("dyn_new", 0);
        runner
            .reload_dynamic_hooks(vec![Box::new(dyn_new)])
            .await;

        // Fire again — static + new dynamic should fire, old dynamic should NOT
        runner.fire_heartbeat_tick().await;
        assert_eq!(static_count.load(Ordering::SeqCst), 2);
        assert_eq!(dyn_old_count.load(Ordering::SeqCst), 1); // unchanged
        assert_eq!(dyn_new_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn static_handlers_unaffected_by_reload() {
        let mut runner = HookRunner::new();
        let (static_hook, static_count) = CountingHook::new("static_hook", 0);
        runner.register(Box::new(static_hook));

        // Reload dynamic with empty vec
        runner.reload_dynamic_hooks(vec![]).await;

        // Static should still fire
        runner.fire_heartbeat_tick().await;
        assert_eq!(static_count.load(Ordering::SeqCst), 1);

        // Reload dynamic with a new hook
        let (dyn_hook, _) = CountingHook::new("dyn_hook", 0);
        runner
            .reload_dynamic_hooks(vec![Box::new(dyn_hook)])
            .await;

        // Static should still fire
        runner.fire_heartbeat_tick().await;
        assert_eq!(static_count.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn empty_reload_clears_dynamic() {
        let runner = HookRunner::new();
        // Load a dynamic hook
        let (dyn_hook, dyn_count) = CountingHook::new("dyn_hook", 0);
        runner
            .reload_dynamic_hooks(vec![Box::new(dyn_hook)])
            .await;
        // Fire once — dynamic should fire
        runner.fire_heartbeat_tick().await;
        assert_eq!(dyn_count.load(Ordering::SeqCst), 1);
        // Reload with empty vec — clears dynamic
        runner.reload_dynamic_hooks(vec![]).await;
        // Fire again — dynamic should NOT fire
        runner.fire_heartbeat_tick().await;
        assert_eq!(dyn_count.load(Ordering::SeqCst), 1); // unchanged
    }

    #[tokio::test]
    async fn cron_delivery_cancelled_by_hook() {
        let mut runner = HookRunner::new();
        runner.register(Box::new(CancelCronDeliveryHook {
            name: "cron_blocker".into(),
            priority: 10,
        }));
        let result = runner
            .run_on_cron_delivery(
                "scheduler".into(),
                "telegram".into(),
                "user_a".into(),
                "daily report".into(),
            )
            .await;
        assert!(result.is_cancel());
    }

    #[tokio::test]
    async fn cron_delivery_passes_through_with_no_hooks() {
        let runner = HookRunner::new();
        let result = runner
            .run_on_cron_delivery(
                "scheduler".into(),
                "telegram".into(),
                "user_a".into(),
                "daily report".into(),
            )
            .await;
        match result {
            HookResult::Continue((s, c, r, ct)) => {
                assert_eq!(s, "scheduler");
                assert_eq!(c, "telegram");
                assert_eq!(r, "user_a");
                assert_eq!(ct, "daily report");
            }
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }

    /// A hook that cancels on_cron_delivery.
    struct CancelCronDeliveryHook {
        name: String,
        priority: i32,
    }

    #[async_trait]
    impl HookHandler for CancelCronDeliveryHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        async fn on_cron_delivery(
            &self,
            _source: String,
            _channel: String,
            _recipient: String,
            _content: String,
        ) -> HookResult<(String, String, String, String)> {
            HookResult::Cancel("cron delivery blocked".into())
        }
    }
}
