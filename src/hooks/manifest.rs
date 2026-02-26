use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// Hook event types matching HookHandler trait methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    // Void hooks (fire-and-forget)
    OnGatewayStart,
    OnGatewayStop,
    OnSessionStart,
    OnSessionEnd,
    OnLlmInput,
    OnLlmOutput,
    OnAfterToolCall,
    OnMessageSent,
    OnHeartbeatTick,
    // Modifying hooks (sequential by priority)
    BeforeModelResolve,
    BeforePromptBuild,
    BeforeLlmCall,
    BeforeToolCall,
    OnMessageReceived,
    OnMessageSending,
    OnCronDelivery,
    OnDocsSyncNotify,
}

impl fmt::Display for HookEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            HookEvent::OnGatewayStart => "on_gateway_start",
            HookEvent::OnGatewayStop => "on_gateway_stop",
            HookEvent::OnSessionStart => "on_session_start",
            HookEvent::OnSessionEnd => "on_session_end",
            HookEvent::OnLlmInput => "on_llm_input",
            HookEvent::OnLlmOutput => "on_llm_output",
            HookEvent::OnAfterToolCall => "on_after_tool_call",
            HookEvent::OnMessageSent => "on_message_sent",
            HookEvent::OnHeartbeatTick => "on_heartbeat_tick",
            HookEvent::BeforeModelResolve => "before_model_resolve",
            HookEvent::BeforePromptBuild => "before_prompt_build",
            HookEvent::BeforeLlmCall => "before_llm_call",
            HookEvent::BeforeToolCall => "before_tool_call",
            HookEvent::OnMessageReceived => "on_message_received",
            HookEvent::OnMessageSending => "on_message_sending",
            HookEvent::OnCronDelivery => "on_cron_delivery",
            HookEvent::OnDocsSyncNotify => "on_docs_sync_notify",
        };
        write!(f, "{}", s)
    }
}

/// Action to execute when a hook fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookAction {
    Shell {
        command: String,
        #[serde(default)]
        timeout_secs: Option<u64>,
        #[serde(default)]
        workdir: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        method: Option<String>,
        #[serde(default)]
        headers: Option<HashMap<String, String>>,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        timeout_secs: Option<u64>,
    },
    PromptInject {
        content: String,
        #[serde(default)]
        position: Option<String>,
    },
}

/// Conditions that filter when a hook should fire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConditions {
    #[serde(default)]
    pub channels: Option<Vec<String>>,
    #[serde(default)]
    pub users: Option<Vec<String>>,
    #[serde(default)]
    pub pattern: Option<String>,
}

/// TOML wrapper: `[hook]` top-level key.
#[derive(Debug, Clone, Deserialize)]
struct HookManifestWrapper {
    hook: HookManifestInner,
}

#[derive(Debug, Clone, Deserialize)]
struct HookManifestInner {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    event: HookEvent,
    #[serde(default)]
    /// Hook execution priority. Higher number = runs first (descending order). Default: 0.
    priority: i32,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    conditions: Option<HookConditions>,
    action: HookAction,
    #[serde(default)]
    skip_security_audit: bool,
}

fn default_enabled() -> bool {
    true
}

/// Parsed and validated HOOK.toml manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookManifest {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub event: HookEvent,
    #[serde(default)]
    /// Hook execution priority. Higher number = runs first (descending order). Default: 0.
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub conditions: Option<HookConditions>,
    pub action: HookAction,
    #[serde(default)]
    pub skip_security_audit: bool,
}
impl HookManifest {
    /// Parse a HOOK.toml file content into a HookManifest.
    pub fn from_toml(content: &str) -> Result<Self> {
        let wrapper: HookManifestWrapper =
            toml::from_str(content).map_err(|e| anyhow::anyhow!("invalid HOOK.toml: {e}"))?;
        let inner = wrapper.hook;
        let manifest = HookManifest {
            name: inner.name,
            description: inner.description,
            version: inner.version,
            event: inner.event,
            priority: inner.priority,
            enabled: inner.enabled,
            conditions: inner.conditions,
            action: inner.action,
            skip_security_audit: inner.skip_security_audit,
        };
        manifest.validate()?;
        Ok(manifest)
    }
    /// Validate the manifest fields.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            bail!("hook name must not be empty");
        }
        match &self.action {
            HookAction::Shell {
                command,
                timeout_secs,
                ..
            } => {
                if command.trim().is_empty() {
                    bail!("shell action command must not be empty");
                }
                if let Some(t) = timeout_secs {
                    if *t == 0 {
                        bail!("shell action timeout_secs must be > 0");
                    }
                }
            }
            HookAction::Http {
                url, timeout_secs, ..
            } => {
                if url.trim().is_empty() {
                    bail!("http action url must not be empty");
                }
                if let Some(t) = timeout_secs {
                    if *t == 0 {
                        bail!("http action timeout_secs must be > 0");
                    }
                }
            }
            HookAction::PromptInject { content, .. } => {
                if content.trim().is_empty() {
                    bail!("prompt_inject action content must not be empty");
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shell_action() {
        let toml = r#"
[hook]
name = "test-hook"
description = "A test hook"
version = "0.1.0"
event = "before_tool_call"
priority = 10

[hook.action.shell]
command = "echo hello"
timeout_secs = 30
workdir = "/tmp"
"#;
        let m = HookManifest::from_toml(toml).unwrap();
        assert_eq!(m.name, "test-hook");
        assert_eq!(m.event, HookEvent::BeforeToolCall);
        assert_eq!(m.priority, 10);
        assert!(m.enabled);
        assert!(!m.skip_security_audit);
        match &m.action {
            HookAction::Shell {
                command,
                timeout_secs,
                workdir,
            } => {
                assert_eq!(command, "echo hello");
                assert_eq!(*timeout_secs, Some(30));
                assert_eq!(workdir.as_deref(), Some("/tmp"));
            }
            _ => panic!("expected shell action"),
        }
    }
    #[test]
    fn parse_http_action() {
        let toml = r#"
[hook]
name = "http-hook"
event = "on_llm_output"
[hook.action.http]
url = "https://example.com/webhook"
method = "POST"
timeout_secs = 10
"#;
        let m = HookManifest::from_toml(toml).unwrap();
        assert_eq!(m.event, HookEvent::OnLlmOutput);
        match &m.action {
            HookAction::Http {
                url,
                method,
                timeout_secs,
                ..
            } => {
                assert_eq!(url, "https://example.com/webhook");
                assert_eq!(method.as_deref(), Some("POST"));
                assert_eq!(*timeout_secs, Some(10));
            }
            _ => panic!("expected http action"),
        }
    }
    #[test]
    fn parse_prompt_inject_action() {
        let toml = r#"
[hook]
name = "inject-hook"
event = "before_prompt_build"
[hook.action.prompt_inject]
content = "Always be helpful"
position = "prepend"
"#;
        let m = HookManifest::from_toml(toml).unwrap();
        assert_eq!(m.event, HookEvent::BeforePromptBuild);
        match &m.action {
            HookAction::PromptInject { content, position } => {
                assert_eq!(content, "Always be helpful");
                assert_eq!(position.as_deref(), Some("prepend"));
            }
            _ => panic!("expected prompt_inject action"),
        }
    }
    #[test]
    fn reject_missing_event() {
        let toml = r#"
[hook]
name = "bad-hook"
[hook.action.shell]
command = "echo hi"
"#;
        assert!(HookManifest::from_toml(toml).is_err());
    }
    #[test]
    fn reject_invalid_event_name() {
        let toml = r#"
[hook]
name = "bad-event"
event = "nonexistent_event"
[hook.action.shell]
command = "echo hi"
"#;
        assert!(HookManifest::from_toml(toml).is_err());
    }
    #[test]
    fn reject_shell_timeout_zero() {
        let toml = r#"
[hook]
name = "zero-timeout"
event = "on_session_start"
[hook.action.shell]
command = "echo hi"
timeout_secs = 0
"#;
        assert!(HookManifest::from_toml(toml).is_err());
    }
    #[test]
    fn parse_with_conditions() {
        let toml = r#"
[hook]
name = "cond-hook"
event = "on_message_received"
[hook.conditions]
channels = ["telegram", "discord"]
users = ["admin"]
pattern = ".*deploy.*"
[hook.action.shell]
command = "echo deploy"
"#;
        let m = HookManifest::from_toml(toml).unwrap();
        let cond = m.conditions.unwrap();
        assert_eq!(cond.channels.unwrap(), vec!["telegram", "discord"]);
        assert_eq!(cond.users.unwrap(), vec!["admin"]);
        assert_eq!(cond.pattern.unwrap(), ".*deploy.*");
    }
    #[test]
    fn defaults_applied() {
        let toml = r#"
[hook]
name = "minimal"
event = "on_heartbeat_tick"
[hook.action.shell]
command = "echo tick"
"#;
        let m = HookManifest::from_toml(toml).unwrap();
        assert_eq!(m.priority, 0);
        assert!(m.enabled);
        assert!(!m.skip_security_audit);
        assert!(m.description.is_none());
        assert!(m.version.is_none());
        assert!(m.conditions.is_none());
    }
    #[test]
    fn display_hook_event() {
        assert_eq!(HookEvent::OnGatewayStart.to_string(), "on_gateway_start");
        assert_eq!(HookEvent::BeforeToolCall.to_string(), "before_tool_call");
        assert_eq!(
            HookEvent::OnMessageSending.to_string(),
            "on_message_sending"
        );
    }
}
