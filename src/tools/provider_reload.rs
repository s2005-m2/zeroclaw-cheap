use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Tool that allows the agent to hot-reload external provider definitions
/// from `~/.zeroclaw/providers/*.toml` at runtime.
pub struct ProviderReloadTool;

impl ProviderReloadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ProviderReloadTool {
    fn name(&self) -> &str {
        "provider_reload"
    }

    fn description(&self) -> &str {
        "Hot-reload external provider definitions from ~/.zeroclaw/providers/*.toml. \
         Returns a summary of added, removed, and errored providers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let registry = match crate::providers::external_registry() {
            Some(r) => r,
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(
                        "External provider registry not initialized".to_string(),
                    ),
                });
            }
        };

        let result = registry.reload();
        let has_errors = !result.errors.is_empty();

        let output = serde_json::to_string_pretty(&result)
            .unwrap_or_else(|e| format!("Failed to serialize result: {e}"));

        Ok(ToolResult {
            success: !has_errors,
            output,
            error: if has_errors {
                Some(format!("{} error(s) during reload", result.errors.len()))
            } else {
                None
            },
        })
    }
}
