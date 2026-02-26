use super::traits::{Tool, ToolResult};
use crate::config::HooksConfig;
use crate::hooks::audit::audit_hook_directory;
use crate::hooks::manifest::HookManifest;
use async_trait::async_trait;
use regex::Regex;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Tool that lets the agent create lifecycle hooks via HOOK.toml generation.
///
/// Generated hooks are gated by the security audit unless `skip_security_audit`
/// is explicitly `true` in the hooks configuration.
pub struct HookWriteTool {
    hooks_config: HooksConfig,
    workspace_dir: PathBuf,
}

impl HookWriteTool {
    pub fn new(hooks_config: HooksConfig, workspace_dir: PathBuf) -> Self {
        Self {
            hooks_config,
            workspace_dir,
        }
    }

    /// Resolve the hooks directory from config or default to `{workspace}/hooks/`.
    fn hooks_dir(&self) -> PathBuf {
        self.hooks_config
            .hooks_dir
            .clone()
            .unwrap_or_else(|| self.workspace_dir.join("hooks"))
    }
}

/// Validate hook name: alphanumeric + hyphens only, 1-64 chars, no traversal.
fn validate_hook_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("Hook name must be 1-64 characters".into());
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return Err("Hook name must start with an alphanumeric character".into()),
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!(
                "Hook name contains invalid character '{c}': only alphanumeric and hyphens allowed"
            ));
        }
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Hook name must not contain path traversal sequences".into());
    }
    Ok(())
}

/// Escape and quote a string for TOML basic string format.
fn toml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
/// Check a shell command for dangerous patterns.
/// Mirrors the patterns from `crate::hooks::audit` to catch dangerous content
/// before writing to disk.
fn audit_shell_content(command: &str) -> Option<&'static str> {
    static DANGEROUS_PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let patterns = DANGEROUS_PATTERNS.get_or_init(|| {
        vec![
            (Regex::new(r"(?im):\(\)\s*\{\s*:\|:&\s*\};:").expect("regex"), "fork-bomb"),
            (Regex::new(r"(?im)/dev/tcp/").expect("regex"), "reverse-shell-dev-tcp"),
            (Regex::new(r"(?im)\bnc(?:at)?\b[^\n]{0,120}\s-e\b").expect("regex"), "netcat-reverse-shell"),
            (Regex::new(r"(?im)\bbash\s+-i\b").expect("regex"), "interactive-bash-reverse-shell"),
            (Regex::new(r"(?im)\bcurl\b[^\n|]{0,200}\|\s*(?:sh|bash|zsh)\b").expect("regex"), "curl-pipe-shell"),
            (Regex::new(r"(?im)\bwget\b[^\n|]{0,200}\|\s*(?:sh|bash|zsh)\b").expect("regex"), "wget-pipe-shell"),
            (Regex::new(r"(?im)\brm\s+-rf\s+/").expect("regex"), "destructive-rm-rf-root"),
            (Regex::new(r"(?im)\bmkfs(?:\.[a-z0-9]+)?\b").expect("regex"), "filesystem-format"),
            (Regex::new(r"(?im)\bdd\s+if=").expect("regex"), "disk-overwrite-dd"),
        ]
    });
    patterns.iter().find_map(|(re, label)| re.is_match(command).then_some(*label))
}

/// Build a HOOK.toml string from the parsed parameters.
fn build_hook_toml(
    name: &str,
    event: &str,
    action_type: &str,
    command: Option<&str>,
    url: Option<&str>,
    content: Option<&str>,
    description: Option<&str>,
    priority: Option<i32>,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    let mut toml = String::from("[hook]\n");
    toml.push_str(&format!("name = {}\n", toml_quote(name)));
    if let Some(desc) = description {
        toml.push_str(&format!("description = {}\n", toml_quote(desc)));
    }
    toml.push_str(&format!("event = {}\n", toml_quote(event)));
    if let Some(p) = priority {
        toml.push_str(&format!("priority = {p}\n"));
    }
    toml.push('\n');

    match action_type {
        "shell" => {
            let cmd = command.ok_or("'command' is required for shell action")?;
            toml.push_str("[hook.action]\n");
            toml.push_str(&format!("shell.command = {}\n", toml_quote(cmd)));
            if let Some(t) = timeout_secs {
                toml.push_str(&format!("shell.timeout_secs = {t}\n"));
            }
        }
        "http" => {
            let u = url.ok_or("'url' is required for http action")?;
            toml.push_str("[hook.action]\n");
            toml.push_str(&format!("http.url = {}\n", toml_quote(u)));
            if let Some(t) = timeout_secs {
                toml.push_str(&format!("http.timeout_secs = {t}\n"));
            }
        }
        "prompt_inject" => {
            let c = content.ok_or("'content' is required for prompt_inject action")?;
            toml.push_str("[hook.action]\n");
            toml.push_str(&format!("prompt_inject.content = {}\n", toml_quote(c)));
        }
        other => {
            return Err(format!(
                "Unknown action_type '{other}': expected shell, http, or prompt_inject"
            ));
        }
    }

    Ok(toml)
}

#[async_trait]
impl Tool for HookWriteTool {
    fn name(&self) -> &str {
        "hook_write"
    }

    fn description(&self) -> &str {
        "Create a lifecycle hook by generating a HOOK.toml manifest in the hooks directory. Supports shell, http, and prompt_inject action types. Subject to security audit unless skip_security_audit is enabled."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Hook name (alphanumeric and hyphens only)"
                },
                "event": {
                    "type": "string",
                    "description": "Hook event (e.g. on_llm_output, before_tool_call)"
                },
                "action_type": {
                    "type": "string",
                    "enum": ["shell", "http", "prompt_inject"],
                    "description": "Type of action to execute when the hook fires"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command (required when action_type=shell)"
                },
                "url": {
                    "type": "string",
                    "description": "HTTP URL (required when action_type=http)"
                },
                "content": {
                    "type": "string",
                    "description": "Prompt content (required when action_type=prompt_inject)"
                },
                "description": {
                    "type": "string",
                    "description": "Optional human-readable description of the hook"
                },
                "priority": {
                    "type": "integer",
                    "description": "Optional execution priority (lower runs first)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional timeout in seconds for shell/http actions"
                }
            },
            "required": ["name", "event", "action_type"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // Extract required parameters
        let name = match args.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing required parameter 'name'".into()),
                });
            }
        };
        let event = match args.get("event").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing required parameter 'event'".into()),
                });
            }
        };
        let action_type = match args.get("action_type").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing required parameter 'action_type'".into()),
                });
            }
        };

        // Validate hook name
        if let Err(e) = validate_hook_name(name) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e),
            });
        }

        // Extract optional parameters
        let command = args.get("command").and_then(|v| v.as_str());
        let url = args.get("url").and_then(|v| v.as_str());
        let content = args.get("content").and_then(|v| v.as_str());
        let description = args.get("description").and_then(|v| v.as_str());
        let priority = args.get("priority").and_then(|v| v.as_i64()).map(|v| v as i32);
        let timeout_secs = args.get("timeout_secs").and_then(|v| v.as_u64());

        // Build HOOK.toml content
        let toml_content = match build_hook_toml(
            name,
            event,
            action_type,
            command,
            url,
            content,
            description,
            priority,
            timeout_secs,
        ) {
            Ok(t) => t,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e),
                });
            }
        };
        // Validate the generated TOML is parseable
        if let Err(e) = HookManifest::from_toml(&toml_content) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Generated HOOK.toml is invalid: {e}")),
            });
        }
        // Pre-write content audit: check shell commands for dangerous patterns
        if !self.hooks_config.skip_security_audit {
            if let Some(cmd) = command {
                if let Some(pattern) = audit_shell_content(cmd) {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!(
                            "Security audit rejected hook '{}': dangerous shell pattern ({})",
                            name, pattern
                        )),
                    });
                }
            }
        }
        // Create hook directory
        let hooks_dir = self.hooks_dir();
        let hook_dir = hooks_dir.join(name);
        if hook_dir.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Hook '{}' already exists at {}", name, hook_dir.display())),
            });
        }
        if let Err(e) = fs::create_dir_all(&hook_dir) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to create hook directory: {e}")),
            });
        }
        // Write HOOK.toml first so audit can read it
        let toml_path = hook_dir.join("HOOK.toml");
        if let Err(e) = fs::write(&toml_path, &toml_content) {
            // Clean up the directory on write failure
            let _ = fs::remove_dir_all(&hook_dir);
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to write HOOK.toml: {e}")),
            });
        }
        // Security audit gate: run audit on the hook directory
        if !self.hooks_config.skip_security_audit {
            match audit_hook_directory(&hook_dir, false) {
                Ok(report) => {
                    if !report.is_clean() {
                        // Remove the hook directory since audit failed
                        let _ = fs::remove_dir_all(&hook_dir);
                        return Ok(ToolResult {
                            success: false,
                            output: String::new(),
                            error: Some(format!(
                                "Security audit rejected hook '{}': {}",
                                name,
                                report.summary()
                            )),
                        });
                    }
                }
                Err(e) => {
                    let _ = fs::remove_dir_all(&hook_dir);
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("Security audit failed: {e}")),
                    });
                }
            }
        }
        Ok(ToolResult {
            success: true,
            output: json!({
                "created": name,
                "path": hook_dir.display().to_string(),
                "event": event,
                "action_type": action_type
            }).to_string(),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HooksConfig;
    use serde_json::json;

    fn test_tool(dir: &std::path::Path) -> HookWriteTool {
        let config = HooksConfig {
            enabled: true,
            hooks_dir: Some(dir.join("hooks")),
            skip_security_audit: false,
            ..HooksConfig::default()
        };
        HookWriteTool::new(config, dir.to_path_buf())
    }

    fn test_tool_skip_audit(dir: &std::path::Path) -> HookWriteTool {
        let config = HooksConfig {
            enabled: true,
            hooks_dir: Some(dir.join("hooks")),
            skip_security_audit: true,
            ..HooksConfig::default()
        };
        HookWriteTool::new(config, dir.to_path_buf())
    }
    #[tokio::test]
    async fn valid_shell_hook_generates_correct_toml() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool
            .execute(json!({
                "name": "my-hook",
                "event": "on_llm_output",
                "action_type": "shell",
                "command": "echo hello",
                "description": "Test hook",
                "priority": 10,
                "timeout_secs": 30
            }))
            .await
            .unwrap();
        assert!(result.success, "failed: {:?}", result.error);
        let toml_path = dir.path().join("hooks/my-hook/HOOK.toml");
        assert!(toml_path.exists());
        let content = fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("my-hook"));
        assert!(content.contains("on_llm_output"));
        assert!(content.contains("echo hello"));
        assert!(content.contains("Test hook"));
    }
    #[tokio::test]
    async fn invalid_hook_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool
            .execute(json!({
                "name": "bad name!",
                "event": "on_llm_output",
                "action_type": "shell",
                "command": "echo hi"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("invalid character"));
    }
    #[tokio::test]
    async fn dangerous_shell_command_rejected_by_audit() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool
            .execute(json!({
                "name": "evil-hook",
                "event": "on_llm_output",
                "action_type": "shell",
                "command": "rm -rf /"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap_or("").contains("Security audit rejected"));
    }
    #[tokio::test]
    async fn skip_security_audit_bypasses_audit() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool_skip_audit(dir.path());
        let result = tool
            .execute(json!({
                "name": "dangerous-hook",
                "event": "on_llm_output",
                "action_type": "shell",
                "command": "rm -rf /"
            }))
            .await
            .unwrap();
        assert!(result.success, "should bypass audit: {:?}", result.error);
        let toml_path = dir.path().join("hooks/dangerous-hook/HOOK.toml");
        assert!(toml_path.exists());
    }
    #[tokio::test]
    async fn generated_toml_parseable_by_hook_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool
            .execute(json!({
                "name": "parse-test",
                "event": "before_tool_call",
                "action_type": "http",
                "url": "https://example.com/webhook"
            }))
            .await
            .unwrap();
        assert!(result.success, "failed: {:?}", result.error);
        let toml_path = dir.path().join("hooks/parse-test/HOOK.toml");
        let content = fs::read_to_string(&toml_path).unwrap();
        let manifest = HookManifest::from_toml(&content).unwrap();
        assert_eq!(manifest.name, "parse-test");
        assert_eq!(manifest.event.to_string(), "before_tool_call");
    }
    #[tokio::test]
    async fn missing_required_fields_return_error() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        // Missing event
        let r1 = tool
            .execute(json!({
                "name": "no-event",
                "action_type": "shell",
                "command": "echo hi"
            }))
            .await
            .unwrap();
        assert!(!r1.success);
        assert!(r1.error.as_deref().unwrap_or("").contains("event"));
        // Missing action_type
        let r2 = tool
            .execute(json!({
                "name": "no-action",
                "event": "on_llm_output"
            }))
            .await
            .unwrap();
        assert!(!r2.success);
        assert!(r2.error.as_deref().unwrap_or("").contains("action_type"));
        // Missing name
        let r3 = tool
            .execute(json!({
                "event": "on_llm_output",
                "action_type": "shell",
                "command": "echo hi"
            }))
            .await
            .unwrap();
        assert!(!r3.success);
        assert!(r3.error.as_deref().unwrap_or("").contains("name"));
    }
    #[tokio::test]
    async fn prompt_inject_hook_creates_valid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool
            .execute(json!({
                "name": "inject-hook",
                "event": "before_prompt_build",
                "action_type": "prompt_inject",
                "content": "Always be helpful"
            }))
            .await
            .unwrap();
        assert!(result.success, "failed: {:?}", result.error);
        let toml_path = dir.path().join("hooks/inject-hook/HOOK.toml");
        let content = fs::read_to_string(&toml_path).unwrap();
        let manifest = HookManifest::from_toml(&content).unwrap();
        assert_eq!(manifest.name, "inject-hook");
    }
}
