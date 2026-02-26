//! MCP Manage Tool — allows AI to add, remove, and list MCP servers at runtime

use super::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use zeroclaw_mcp::config::McpServerConfig;
use zeroclaw_mcp::registry::McpRegistry;

use crate::security::policy::AutonomyLevel;

pub struct McpManageTool {
    registry: Arc<McpRegistry>,
    autonomy_level: AutonomyLevel,
    config_path: PathBuf,
}

impl McpManageTool {
    pub fn new(
        registry: Arc<McpRegistry>,
        autonomy_level: AutonomyLevel,
        config_path: PathBuf,
    ) -> Self {
        Self {
            registry,
            autonomy_level,
            config_path,
        }
    }
}

#[async_trait]
impl Tool for McpManageTool {
    fn name(&self) -> &str {
        "mcp_manage"
    }

    fn description(&self) -> &str {
        "Manage MCP (Model Context Protocol) servers. Use 'list' to see running servers and their tool counts. Use 'add' to install a new MCP server (requires name, command, optional args and env). Use 'remove' to uninstall a server. Common MCP servers: filesystem (npx -y @modelcontextprotocol/server-filesystem /path), git (npx -y @modelcontextprotocol/server-git), fetch (npx -y @modelcontextprotocol/server-fetch), postgres (npx -y @modelcontextprotocol/server-postgres). After adding a server, its tools become available automatically. Requires Full autonomy level for add/remove."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "remove", "list"],
                    "description": "Action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Server name (required for add/remove)"
                },
                "command": {
                    "type": "string",
                    "description": "Server command (required for add)"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command arguments"
                },
                "env": {
                    "type": "object",
                    "description": "Environment variables"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args["action"].as_str().unwrap_or("");

        match action {
            "list" => {
                let servers = self.registry.list_servers().await;
                let list: Vec<_> = servers
                    .iter()
                    .map(|(name, count)| json!({"name": name, "tool_count": count}))
                    .collect();
                Ok(ToolResult {
                    success: true,
                    output: serde_json::to_string_pretty(&list)?,
                    error: None,
                })
            }
            "add" => {
                if self.autonomy_level != AutonomyLevel::Full {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some("MCP server addition requires Full autonomy level".to_string()),
                    });
                }
                let name = args["name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'name' is required for add"))?;
                let command = args["command"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'command' is required for add"))?;
                let cmd_args: Vec<String> = args["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let env: HashMap<String, String> = args["env"]
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();

                let config = McpServerConfig {
                    name: name.to_string(),
                    command: command.to_string(),
                    args: cmd_args,
                    env,
                };
                let tools = self.registry.add_server(config).await?;
                Ok(ToolResult {
                    success: true,
                    output: format!("Added MCP server '{}' with {} tools", name, tools.len()),
                    error: None,
                })
            }
            "remove" => {
                let name = args["name"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'name' is required for remove"))?;
                self.registry.remove_server(name).await?;
                Ok(ToolResult {
                    success: true,
                    output: format!("Removed MCP server '{}'", name),
                    error: None,
                })
            }
            _ => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Unknown action: '{}'", action)),
            }),
        }
    }
}
