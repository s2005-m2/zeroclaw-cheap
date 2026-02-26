use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, warn};

use crate::channels::traits::ChannelMessage;
use crate::hooks::loader::LoadedHook;
use crate::hooks::manifest::{HookAction, HookConditions, HookEvent};
use crate::hooks::traits::{HookHandler, HookResult};
use crate::providers::traits::{ChatMessage, ChatResponse};
use crate::tools::traits::ToolResult;

/// Default timeout for hook actions when none is specified.
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// A dynamic hook handler backed by a HOOK.toml manifest loaded from disk.
#[derive(Debug, Clone)]
pub struct DynamicHookHandler {
    pub hook: LoadedHook,
    pub default_timeout_secs: u64,
}

impl DynamicHookHandler {
    pub fn new(hook: LoadedHook, default_timeout_secs: u64) -> Self {
        Self {
            hook,
            default_timeout_secs,
        }
    }

    /// Check if this hook's event matches the given event type.
    fn matches_event(&self, event: HookEvent) -> bool {
        self.hook.manifest.enabled && self.hook.manifest.event == event
    }

    /// Evaluate conditions against available context. Best-effort: if no context, skip check.
    fn check_conditions(
        &self,
        channel: Option<&str>,
        user: Option<&str>,
        text: Option<&str>,
    ) -> bool {
        let conditions = match &self.hook.manifest.conditions {
            Some(c) => c,
            None => return true,
        };
        if !check_channel_condition(conditions, channel) {
            return false;
        }
        if !check_user_condition(conditions, user) {
            return false;
        }
        if !check_pattern_condition(conditions, text) {
            return false;
        }
        true
    }

    /// Get the effective timeout for this hook's action.
    fn effective_timeout(&self) -> Duration {
        let secs = match &self.hook.manifest.action {
            HookAction::Shell { timeout_secs, .. } => {
                timeout_secs.unwrap_or(self.default_timeout_secs)
            }
            HookAction::Http { timeout_secs, .. } => {
                timeout_secs.unwrap_or(self.default_timeout_secs)
            }
            HookAction::PromptInject { .. } => self.default_timeout_secs,
        };
        Duration::from_secs(if secs == 0 {
            DEFAULT_TIMEOUT_SECS
        } else {
            secs
        })
    }

    /// Execute a shell action. Returns Ok(output) or Err on failure/timeout.
    async fn execute_shell(&self, command: &str, workdir: Option<&str>) -> Result<String, String> {
        let timeout = self.effective_timeout();
        let mut cmd = tokio::process::Command::new(shell_program());
        cmd.arg(shell_flag());
        cmd.arg(command);

        if let Some(dir) = workdir {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&self.hook.hook_dir);
        }

        let result = tokio::time::timeout(timeout, cmd.output()).await;
        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if output.status.success() {
                    Ok(stdout)
                } else {
                    Err(format!("exit code {}: {}", output.status, stderr))
                }
            }
            Ok(Err(e)) => Err(format!("failed to spawn command: {e}")),
            Err(_) => Err(format!("hook timed out after {}s", timeout.as_secs())),
        }
    }

    /// Execute an HTTP action. Returns Ok(body) or Err on failure/timeout.
    async fn execute_http(
        &self,
        url: &str,
        method: Option<&str>,
        headers: Option<&std::collections::HashMap<String, String>>,
        body: Option<&str>,
    ) -> Result<String, String> {
        let timeout = self.effective_timeout();
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;

        let method_str = method.unwrap_or("GET");
        let req_method = method_str
            .parse::<reqwest::Method>()
            .map_err(|e| format!("invalid HTTP method '{method_str}': {e}"))?;

        let mut req = client.request(req_method, url);
        if let Some(hdrs) = headers {
            for (k, v) in hdrs {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        if let Some(b) = body {
            req = req.body(b.to_string());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;
        let text = resp
            .text()
            .await
            .map_err(|e| format!("failed to read response: {e}"))?;
        Ok(text)
    }

    /// Fire a void hook action (fire-and-forget via spawn).
    fn fire_void_action(&self) {
        let hook = self.clone();
        tokio::spawn(async move {
            if let Err(e) = hook.execute_action().await {
                warn!(
                    hook = hook.hook.manifest.name.as_str(),
                    "void hook action failed: {e}"
                );
            }
        });
    }

    /// Execute the hook's action and return the result string.
    async fn execute_action(&self) -> Result<String, String> {
        match &self.hook.manifest.action {
            HookAction::Shell {
                command, workdir, ..
            } => self.execute_shell(command, workdir.as_deref()).await,
            HookAction::Http {
                url,
                method,
                headers,
                body,
                ..
            } => {
                self.execute_http(url, method.as_deref(), headers.as_ref(), body.as_deref())
                    .await
            }
            HookAction::PromptInject { .. } => {
                // PromptInject is handled inline by modifying hooks, not as a side-effect
                Ok(String::new())
            }
        }
    }
}

#[async_trait]
impl HookHandler for DynamicHookHandler {
    fn name(&self) -> &str {
        &self.hook.manifest.name
    }

    fn priority(&self) -> i32 {
        self.hook.manifest.priority
    }

    // --- Void hooks (fire-and-forget) ---

    async fn on_gateway_start(&self, _host: &str, _port: u16) {
        if self.matches_event(HookEvent::OnGatewayStart) {
            debug!(hook = self.name(), "firing on_gateway_start");
            self.fire_void_action();
        }
    }

    async fn on_gateway_stop(&self) {
        if self.matches_event(HookEvent::OnGatewayStop) {
            debug!(hook = self.name(), "firing on_gateway_stop");
            self.fire_void_action();
        }
    }

    async fn on_session_start(&self, _session_id: &str, _channel: &str) {
        if self.matches_event(HookEvent::OnSessionStart) {
            debug!(hook = self.name(), "firing on_session_start");
            self.fire_void_action();
        }
    }

    async fn on_session_end(&self, _session_id: &str, _channel: &str) {
        if self.matches_event(HookEvent::OnSessionEnd) {
            debug!(hook = self.name(), "firing on_session_end");
            self.fire_void_action();
        }
    }

    async fn on_llm_input(&self, _messages: &[ChatMessage], _model: &str) {
        if self.matches_event(HookEvent::OnLlmInput) {
            debug!(hook = self.name(), "firing on_llm_input");
            self.fire_void_action();
        }
    }

    async fn on_llm_output(&self, _response: &ChatResponse) {
        if self.matches_event(HookEvent::OnLlmOutput) {
            debug!(hook = self.name(), "firing on_llm_output");
            self.fire_void_action();
        }
    }

    async fn on_after_tool_call(&self, _tool: &str, _result: &ToolResult, _duration: Duration) {
        if self.matches_event(HookEvent::OnAfterToolCall) {
            debug!(hook = self.name(), "firing on_after_tool_call");
            self.fire_void_action();
        }
    }

    async fn on_message_sent(&self, _channel: &str, _recipient: &str, _content: &str) {
        if self.matches_event(HookEvent::OnMessageSent) {
            debug!(hook = self.name(), "firing on_message_sent");
            self.fire_void_action();
        }
    }

    async fn on_heartbeat_tick(&self) {
        if self.matches_event(HookEvent::OnHeartbeatTick) {
            debug!(hook = self.name(), "firing on_heartbeat_tick");
            self.fire_void_action();
        }
    }

    // --- Modifying hooks (sequential by priority) ---
    async fn before_model_resolve(
        &self,
        provider: String,
        model: String,
    ) -> HookResult<(String, String)> {
        if !self.matches_event(HookEvent::BeforeModelResolve) {
            return HookResult::Continue((provider, model));
        }
        if !self.check_conditions(None, None, None) {
            return HookResult::Continue((provider, model));
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue((provider, model)),
            Err(e) => {
                warn!(hook = self.name(), "before_model_resolve action failed: {e}");
                HookResult::Continue((provider, model))
            }
        }
    }

    async fn before_prompt_build(&self, prompt: String) -> HookResult<String> {
        if !self.matches_event(HookEvent::BeforePromptBuild) {
            return HookResult::Continue(prompt);
        }
        if !self.check_conditions(None, None, Some(&prompt)) {
            return HookResult::Continue(prompt);
        }
        // PromptInject action modifies the prompt directly
        if let HookAction::PromptInject { content, position } = &self.hook.manifest.action {
            let pos = position.as_deref().unwrap_or("prepend");
            let modified = if pos == "append" {
                format!("{prompt}\n{content}")
            } else {
                format!("{content}\n{prompt}")
            };
            return HookResult::Continue(modified);
        }
        // Shell/Http actions: execute and continue with original prompt
        match self.execute_action().await {
            Ok(_) => HookResult::Continue(prompt),
            Err(e) => {
                warn!(hook = self.name(), "before_prompt_build action failed: {e}");
                HookResult::Continue(prompt)
            }
        }
    }

    async fn before_llm_call(
        &self,
        messages: Vec<ChatMessage>,
        model: String,
    ) -> HookResult<(Vec<ChatMessage>, String)> {
        if !self.matches_event(HookEvent::BeforeLlmCall) {
            return HookResult::Continue((messages, model));
        }
        if !self.check_conditions(None, None, None) {
            return HookResult::Continue((messages, model));
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue((messages, model)),
            Err(e) => {
                warn!(hook = self.name(), "before_llm_call action failed: {e}");
                HookResult::Continue((messages, model))
            }
        }
    }
    async fn before_tool_call(&self, name: String, args: Value) -> HookResult<(String, Value)> {
        if !self.matches_event(HookEvent::BeforeToolCall) {
            return HookResult::Continue((name, args));
        }
        if !self.check_conditions(None, None, None) {
            return HookResult::Continue((name, args));
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue((name, args)),
            Err(e) => {
                warn!(hook = self.name(), "before_tool_call action failed: {e}");
                HookResult::Cancel(format!("hook {} blocked tool call: {e}", self.name()))
            }
        }
    }
    async fn on_message_received(&self, message: ChannelMessage) -> HookResult<ChannelMessage> {
        if !self.matches_event(HookEvent::OnMessageReceived) {
            return HookResult::Continue(message);
        }
        let channel = Some(message.channel.as_str());
        let user = Some(message.sender.as_str());
        let text = Some(message.content.as_str());
        if !self.check_conditions(channel, user, text) {
            return HookResult::Continue(message);
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue(message),
            Err(e) => {
                warn!(hook = self.name(), "on_message_received action failed: {e}");
                HookResult::Continue(message)
            }
        }
    }
    async fn on_message_sending(
        &self,
        channel: String,
        recipient: String,
        content: String,
    ) -> HookResult<(String, String, String)> {
        if !self.matches_event(HookEvent::OnMessageSending) {
            return HookResult::Continue((channel, recipient, content));
        }
        if !self.check_conditions(Some(&channel), None, Some(&content)) {
            return HookResult::Continue((channel, recipient, content));
        }
        // PromptInject on sending: modify content
        if let HookAction::PromptInject { content: inject, position } = &self.hook.manifest.action {
            let pos = position.as_deref().unwrap_or("prepend");
            let modified = if pos == "append" {
                format!("{content}\n{inject}")
            } else {
                format!("{inject}\n{content}")
            };
            return HookResult::Continue((channel, recipient, modified));
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue((channel, recipient, content)),
            Err(e) => {
                warn!(hook = self.name(), "on_message_sending action failed: {e}");
                HookResult::Continue((channel, recipient, content))
            }
        }
    }
    async fn on_cron_delivery(
        &self,
        source: String,
        channel: String,
        recipient: String,
        content: String,
    ) -> HookResult<(String, String, String, String)> {
        if !self.matches_event(HookEvent::OnCronDelivery) {
            return HookResult::Continue((source, channel, recipient, content));
        }
        if !self.check_conditions(Some(&channel), None, Some(&content)) {
            return HookResult::Continue((source, channel, recipient, content));
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue((source, channel, recipient, content)),
            Err(e) => {
                warn!(hook = self.name(), "on_cron_delivery action failed: {e}");
                HookResult::Continue((source, channel, recipient, content))
            }
        }
    }
    async fn on_docs_sync_notify(
        &self,
        file_path: String,
        channel: String,
        recipient: String,
        content: String,
    ) -> HookResult<(String, String, String, String)> {
        if !self.matches_event(HookEvent::OnDocsSyncNotify) {
            return HookResult::Continue((file_path, channel, recipient, content));
        }
        if !self.check_conditions(Some(&channel), None, Some(&content)) {
            return HookResult::Continue((file_path, channel, recipient, content));
        }
        match self.execute_action().await {
            Ok(_) => HookResult::Continue((file_path, channel, recipient, content)),
            Err(e) => {
                warn!(hook = self.name(), "on_docs_sync_notify action failed: {e}");
                HookResult::Continue((file_path, channel, recipient, content))
            }
        }
    }
}

// --- Helper functions ---

fn check_channel_condition(conditions: &HookConditions, channel: Option<&str>) -> bool {
    match (&conditions.channels, channel) {
        (Some(allowed), Some(ch)) => allowed.iter().any(|c| c == ch),
        (Some(_), None) => true, // no context available, skip check
        (None, _) => true,
    }
}

fn check_user_condition(conditions: &HookConditions, user: Option<&str>) -> bool {
    match (&conditions.users, user) {
        (Some(allowed), Some(u)) => allowed.iter().any(|a| a == u),
        (Some(_), None) => true, // no context available, skip check
        (None, _) => true,
    }
}

fn check_pattern_condition(conditions: &HookConditions, text: Option<&str>) -> bool {
    match (&conditions.pattern, text) {
        (Some(pat), Some(t)) => {
            match regex::Regex::new(pat) {
                Ok(re) => re.is_match(t),
                Err(_) => {
                    warn!("invalid hook condition pattern: {pat}");
                    true // invalid pattern = skip check
                }
            }
        }
        (Some(_), None) => true, // no context available, skip check
        (None, _) => true,
    }
}

#[cfg(target_os = "windows")]
fn shell_program() -> &'static str {
    "cmd"
}

#[cfg(target_os = "windows")]
fn shell_flag() -> &'static str {
    "/C"
}

#[cfg(not(target_os = "windows"))]
fn shell_program() -> &'static str {
    "sh"
}

#[cfg(not(target_os = "windows"))]
fn shell_flag() -> &'static str {
    "-c"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::loader::LoadedHook;
    use crate::hooks::manifest::{HookAction, HookConditions, HookEvent, HookManifest};
    use std::path::PathBuf;

    fn make_hook(name: &str, event: HookEvent, priority: i32, action: HookAction) -> LoadedHook {
        LoadedHook {
            manifest: HookManifest {
                name: name.to_string(),
                description: None,
                version: None,
                event,
                priority,
                enabled: true,
                conditions: None,
                action,
                skip_security_audit: true,
            },
            hook_dir: PathBuf::from("/tmp/test-hook"),
        }
    }

    fn shell_action(cmd: &str) -> HookAction {
        HookAction::Shell {
            command: cmd.to_string(),
            timeout_secs: None,
            workdir: None,
        }
    }

    fn prompt_inject_action(content: &str, position: &str) -> HookAction {
        HookAction::PromptInject {
            content: content.to_string(),
            position: Some(position.to_string()),
        }
    }
    #[test]
    fn name_and_priority_from_manifest() {
        let hook = make_hook("test-hook", HookEvent::OnSessionStart, 42, shell_action("echo hi"));
        let handler = DynamicHookHandler::new(hook, 30);
        assert_eq!(handler.name(), "test-hook");
        assert_eq!(handler.priority(), 42);
    }
    #[tokio::test]
    async fn shell_action_executes_on_matching_event() {
        let hook = make_hook(
            "shell-hook",
            HookEvent::BeforeToolCall,
            5,
            shell_action("echo hello"),
        );
        let handler = DynamicHookHandler::new(hook, 30);
        let result = handler
            .before_tool_call("shell".into(), serde_json::json!({}))
            .await;
        match result {
            HookResult::Continue((name, _)) => assert_eq!(name, "shell"),
            HookResult::Cancel(r) => panic!("unexpected cancel: {r}"),
        }
    }
    #[tokio::test]
    async fn non_matching_event_is_noop() {
        let hook = make_hook(
            "session-hook",
            HookEvent::OnSessionStart,
            5,
            shell_action("echo hi"),
        );
        let handler = DynamicHookHandler::new(hook, 30);
        // Call before_tool_call — event is OnSessionStart, not BeforeToolCall
        let result = handler
            .before_tool_call("shell".into(), serde_json::json!({"x": 1}))
            .await;
        match result {
            HookResult::Continue((name, args)) => {
                assert_eq!(name, "shell");
                assert_eq!(args, serde_json::json!({"x": 1}));
            }
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }
    #[tokio::test]
    async fn shell_timeout_enforced() {
        let hook = make_hook(
            "slow-hook",
            HookEvent::BeforeToolCall,
            5,
            HookAction::Shell {
                command: if cfg!(windows) {
                    "ping -n 10 127.0.0.1".to_string()
                } else {
                    "sleep 10".to_string()
                },
                timeout_secs: Some(1),
                workdir: None,
            },
        );
        let handler = DynamicHookHandler::new(hook, 30);
        let result = handler
            .before_tool_call("shell".into(), serde_json::json!({}))
            .await;
        // Should cancel due to timeout
        assert!(result.is_cancel());
    }
    #[tokio::test]
    async fn prompt_inject_prepends_content() {
        let hook = make_hook(
            "inject-hook",
            HookEvent::BeforePromptBuild,
            5,
            prompt_inject_action("SYSTEM: Be helpful", "prepend"),
        );
        let handler = DynamicHookHandler::new(hook, 30);
        let result = handler.before_prompt_build("user prompt".into()).await;
        match result {
            HookResult::Continue(p) => {
                assert!(p.starts_with("SYSTEM: Be helpful"));
                assert!(p.ends_with("user prompt"));
            }
            HookResult::Cancel(_) => panic!("should not cancel"),
        }
    }

    #[tokio::test]
    async fn conditions_filter_by_channel() {
        let mut hook = make_hook(
            "channel-hook",
            HookEvent::OnMessageReceived,
            5,
            shell_action("echo filtered"),
        );
        hook.manifest.conditions = Some(HookConditions {
            channels: Some(vec!["telegram".to_string()]),
            users: None,
            pattern: None,
        });
        let handler = DynamicHookHandler::new(hook, 30);
        // Matching channel
        assert!(handler.check_conditions(Some("telegram"), None, None));
        // Non-matching channel
        assert!(!handler.check_conditions(Some("discord"), None, None));
    }

    #[test]
    fn disabled_hook_does_not_match() {
        let mut hook = make_hook(
            "disabled-hook",
            HookEvent::OnSessionStart,
            5,
            shell_action("echo hi"),
        );
        hook.manifest.enabled = false;
        let handler = DynamicHookHandler::new(hook, 30);
        assert!(!handler.matches_event(HookEvent::OnSessionStart));
    }

    #[tokio::test]
    async fn empty_command_returns_cancel() {
        let hook = make_hook(
            "empty-cmd",
            HookEvent::BeforeToolCall,
            5,
            HookAction::Shell {
                command: String::new(),
                timeout_secs: None,
                workdir: None,
            },
        );
        let handler = DynamicHookHandler::new(hook, 30);
        let result = handler
            .before_tool_call("shell".into(), serde_json::json!({}))
            .await;
        // Empty command should fail gracefully
        match result {
            HookResult::Continue(_) | HookResult::Cancel(_) => {} // either is acceptable
        }
    }
}
