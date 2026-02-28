use super::traits::{Tool, ToolResult};
use crate::config::HooksConfig;
use crate::hooks::reload::write_reload_stamp;
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

/// Tool that triggers a hot-reload of dynamic hooks from the hooks directory.
///
/// The agent creates/edits HOOK.toml files directly via file_write, then calls
/// this tool to signal the runtime to pick up changes.
pub struct HookReloadTool {
    hooks_config: HooksConfig,
    workspace_dir: PathBuf,
}

impl HookReloadTool {
    pub fn new(hooks_config: HooksConfig, workspace_dir: PathBuf) -> Self {
        Self {
            hooks_config,
            workspace_dir,
        }
    }
}

#[async_trait]
impl Tool for HookReloadTool {
    fn name(&self) -> &str {
        "hook_reload"
    }

    fn description(&self) -> &str {
        "Trigger a hot-reload of lifecycle hooks. Call after creating or editing HOOK.toml files in the hooks directory. The runtime will re-scan, validate, and activate all valid hooks."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        // Verify hooks directory exists
        let hooks_dir = self
            .hooks_config
            .hooks_dir
            .clone()
            .unwrap_or_else(|| self.workspace_dir.join("hooks"));

        if !hooks_dir.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Hooks directory does not exist: {}",
                    hooks_dir.display()
                )),
            });
        }

        // Write reload stamp to trigger the runtime reload cycle
        if let Err(e) = write_reload_stamp(&self.workspace_dir) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to write reload stamp: {e}")),
            });
        }

        Ok(ToolResult {
            success: true,
            output: json!({
                "status": "reload_triggered",
                "hooks_dir": hooks_dir.display().to_string()
            })
            .to_string(),
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HooksConfig;
    use serde_json::json;

    fn test_tool(dir: &std::path::Path) -> HookReloadTool {
        let config = HooksConfig {
            enabled: true,
            hooks_dir: Some(dir.join("hooks")),
            ..HooksConfig::default()
        };
        HookReloadTool::new(config, dir.to_path_buf())
    }

    #[tokio::test]
    async fn reload_succeeds_when_hooks_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("hooks")).unwrap();
        let tool = test_tool(dir.path());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.success, "failed: {:?}", result.error);
        assert!(result.output.contains("reload_triggered"));
    }

    #[tokio::test]
    async fn reload_fails_when_hooks_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("does not exist"));
    }

    #[tokio::test]
    async fn reload_writes_stamp_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("hooks")).unwrap();
        let tool = test_tool(dir.path());
        tool.execute(json!({})).await.unwrap();
        let stamp = dir.path().join(".hooks-reload-stamp");
        assert!(stamp.exists());
    }
}
