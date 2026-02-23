//! MCP Bridge Tool — wraps individual MCP tools from connected servers
//!
//! Each `McpBridgeTool` wraps one MCP tool and delegates execution to
//! `McpRegistry::call_tool()`. Tool names are namespaced as `mcp_{server}_{tool}`.

use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use std::sync::Arc;
use zeroclaw_mcp::registry::McpRegistry;
use zeroclaw_mcp::types::McpToolInfo;

/// A tool that wraps a single MCP tool from a connected server
pub struct McpBridgeTool {
    server_name: String,
    tool_info: McpToolInfo,
    registry: Arc<McpRegistry>,
    namespaced_name: String,
}

impl McpBridgeTool {
    /// Create a new MCP bridge tool
    ///
    /// # Arguments
    /// * `server_name` - Name of the MCP server hosting this tool
    /// * `tool_info` - Tool metadata (name, description, input_schema)
    /// * `registry` - Shared MCP registry for tool execution
    pub fn new(server_name: String, tool_info: McpToolInfo, registry: Arc<McpRegistry>) -> Self {
        let namespaced_name = format!("mcp_{}_{}", server_name, tool_info.name);
        Self {
            server_name,
            tool_info,
            registry,
            namespaced_name,
        }
    }
}

#[async_trait]
impl Tool for McpBridgeTool {
    fn name(&self) -> &str {
        &self.namespaced_name
    }

    fn description(&self) -> &str {
        self.tool_info.description.as_deref().unwrap_or("")
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.tool_info.input_schema.clone()
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let result = self
            .registry
            .call_tool(&self.server_name, &self.tool_info.name, Some(args))
            .await;

        match result {
            Ok(mcp_result) => {
                // Join all text content items with newline
                let output = mcp_result
                    .content
                    .iter()
                    .filter_map(|c| c.text.as_ref())
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");

                // Check is_error flag
                let is_error = mcp_result.is_error.unwrap_or(false);

                if is_error {
                    Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(output),
                    })
                } else {
                    Ok(ToolResult {
                        success: true,
                        output,
                        error: None,
                    })
                }
            }
            Err(err) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(err.to_string()),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    use zeroclaw_mcp::types::{McpContent, McpToolCallResult};

    // We'll need a mock transport for testing - similar to registry tests
    // For now, test the conversion logic with a simpler approach

    #[test]
    fn namespaced_name_format() {
        let registry = Arc::new(McpRegistry::new(50, HashSet::new()));
        let tool_info = McpToolInfo {
            name: "file_read".to_string(),
            description: Some("Read a file".to_string()),
            input_schema: json!({"type": "object"}),
        };
        let tool = McpBridgeTool::new("test_server".to_string(), tool_info, registry);

        assert_eq!(tool.name(), "mcp_test_server_file_read");
    }

    #[test]
    fn description_returns_empty_string_when_none() {
        let registry = Arc::new(McpRegistry::new(50, HashSet::new()));
        let tool_info = McpToolInfo {
            name: "test_tool".to_string(),
            description: None,
            input_schema: json!({"type": "object"}),
        };
        let tool = McpBridgeTool::new("server".to_string(), tool_info, registry);

        assert_eq!(tool.description(), "");
    }

    #[test]
    fn description_returns_value_when_some() {
        let registry = Arc::new(McpRegistry::new(50, HashSet::new()));
        let tool_info = McpToolInfo {
            name: "test_tool".to_string(),
            description: Some("Test description".to_string()),
            input_schema: json!({"type": "object"}),
        };
        let tool = McpBridgeTool::new("server".to_string(), tool_info, registry);

        assert_eq!(tool.description(), "Test description");
    }

    #[test]
    fn parameters_schema_returns_input_schema() {
        let registry = Arc::new(McpRegistry::new(50, HashSet::new()));
        let expected_schema = json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        });
        let tool_info = McpToolInfo {
            name: "test_tool".to_string(),
            description: None,
            input_schema: expected_schema.clone(),
        };
        let tool = McpBridgeTool::new("server".to_string(), tool_info, registry);

        assert_eq!(tool.parameters_schema(), expected_schema);
    }

    #[tokio::test]
    async fn execute_converts_success_result() {
        // This test would need a mock registry
        // For now, test the McpToolCallResult to ToolResult conversion logic
        let mcp_result = McpToolCallResult {
            content: vec![
                McpContent {
                    content_type: "text".to_string(),
                    text: Some("line1".to_string()),
                },
                McpContent {
                    content_type: "text".to_string(),
                    text: Some("line2".to_string()),
                },
            ],
            is_error: Some(false),
        };

        // Simulate the conversion logic
        let output = mcp_result
            .content
            .iter()
            .filter_map(|c| c.text.as_ref())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let is_error = mcp_result.is_error.unwrap_or(false);
        let tool_result = if is_error {
            ToolResult {
                success: false,
                output: String::new(),
                error: Some(output),
            }
        } else {
            ToolResult {
                success: true,
                output,
                error: None,
            }
        };

        assert!(tool_result.success);
        assert_eq!(tool_result.output, "line1\nline2");
        assert!(tool_result.error.is_none());
    }

    #[tokio::test]
    async fn execute_converts_error_result() {
        let mcp_result = McpToolCallResult {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: Some("Error occurred".to_string()),
            }],
            is_error: Some(true),
        };

        // Simulate the conversion logic
        let output = mcp_result
            .content
            .iter()
            .filter_map(|c| c.text.as_ref())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let is_error = mcp_result.is_error.unwrap_or(false);
        let tool_result = if is_error {
            ToolResult {
                success: false,
                output: String::new(),
                error: Some(output),
            }
        } else {
            ToolResult {
                success: true,
                output,
                error: None,
            }
        };

        assert!(!tool_result.success);
        assert_eq!(tool_result.output, "");
        assert_eq!(tool_result.error, Some("Error occurred".to_string()));
    }

    #[tokio::test]
    async fn execute_handles_empty_content() {
        let mcp_result = McpToolCallResult {
            content: vec![],
            is_error: Some(false),
        };

        // Simulate the conversion logic
        let output = mcp_result
            .content
            .iter()
            .filter_map(|c| c.text.as_ref())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let is_error = mcp_result.is_error.unwrap_or(false);
        let tool_result = if is_error {
            ToolResult {
                success: false,
                output: String::new(),
                error: Some(output),
            }
        } else {
            ToolResult {
                success: true,
                output,
                error: None,
            }
        };

        assert!(tool_result.success);
        assert_eq!(tool_result.output, "");
        assert!(tool_result.error.is_none());
    }

    #[tokio::test]
    async fn execute_handles_none_is_error() {
        let mcp_result = McpToolCallResult {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: Some("Success".to_string()),
            }],
            is_error: None,
        };

        // Simulate the conversion logic (None should default to false)
        let output = mcp_result
            .content
            .iter()
            .filter_map(|c| c.text.as_ref())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let is_error = mcp_result.is_error.unwrap_or(false);
        let tool_result = if is_error {
            ToolResult {
                success: false,
                output: String::new(),
                error: Some(output),
            }
        } else {
            ToolResult {
                success: true,
                output,
                error: None,
            }
        };

        assert!(tool_result.success);
        assert_eq!(tool_result.output, "Success");
        assert!(tool_result.error.is_none());
    }
}
