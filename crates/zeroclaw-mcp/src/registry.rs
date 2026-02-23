//! MCP server registry for managing multiple MCP server connections
//!
//! This module provides a thread-safe registry for managing multiple MCP servers,
//! their discovered tools, and tool execution routing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::client::McpClient;
use crate::config::McpServerConfig;
use crate::transport::StdioTransport;
use crate::types::McpToolCallResult;
use crate::types::McpToolInfo;

/// Internal state for a connected MCP server
struct McpServerState {
    client: McpClient,
    tools: Vec<McpToolInfo>,
    config: McpServerConfig,
}

/// Registry for managing multiple MCP server connections
pub struct McpRegistry {
    servers: Arc<RwLock<HashMap<String, McpServerState>>>,
    tool_cap: usize,
    builtin_tool_names: HashSet<String>,
}

impl McpRegistry {
    /// Create a new MCP registry with the given tool capacity and builtin tool names
    ///
    /// # Arguments
    /// * `tool_cap` - Maximum number of MCP tools allowed across all servers
    /// * `builtin_tool_names` - Set of builtin tool names that cannot be shadowed
    pub fn new(tool_cap: usize, builtin_tool_names: HashSet<String>) -> Self {
        Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            tool_cap,
            builtin_tool_names,
        }
    }

    /// Add a new MCP server to the registry
    ///
    /// Spawns an StdioTransport, connects an McpClient, lists tools, and stores state.
    /// Rejects if tool names collide with builtins or if total tools exceed cap.
    ///
    /// # Arguments
    /// * `config` - Server configuration with name, command, args, and env
    ///
    /// # Returns
    /// * `Ok(Vec<McpToolInfo>)` - List of discovered tools from the server
    /// * `Err` - Failed to connect, or tool name collision, or cap exceeded
    pub async fn add_server(&self, config: McpServerConfig) -> Result<Vec<McpToolInfo>> {
        let server_name = config.name.clone();

        info!("Adding MCP server: {}", server_name);


        let mut transport = StdioTransport::new(&config.command, &config.args, &config.env)
            .await
            .with_context(|| format!("Failed to spawn MCP server '{}'", server_name))?;

        let mut client = McpClient::connect(Box::new(transport))
            .await
            .with_context(|| format!("Failed to connect to MCP server '{}'", server_name))?;


        let tools = client
            .list_tools()
            .await
            .with_context(|| format!("Failed to list tools from MCP server '{}'", server_name))?;

        debug!(
            "MCP server '{}' advertised {} tools",
            server_name,
            tools.len()
        );


        self.validate_tools(&tools, &server_name).await?;


        {
            let servers = self.servers.read().await;
            let current_tool_count = servers.values().map(|s| s.tools.len()).sum::<usize>();
            if current_tool_count + tools.len() > self.tool_cap {

                let _ = client.close().await;
                anyhow::bail!(
                    "Adding server '{}' would exceed tool cap ({}). Current: {}, New: {}, Cap: {}",
                    server_name,
                    self.tool_cap,
                    current_tool_count,
                    tools.len(),
                    self.tool_cap
                );
            }
        }


        let mut servers = self.servers.write().await;
        servers.insert(
            server_name.clone(),
            McpServerState {
                client,
                tools: tools.clone(),
                config,
            },
        );

        info!(
            "MCP server '{}' added successfully with {} tools",
            server_name,
            tools.len()
        );

        Ok(tools)
    }

    /// Internal method to add a server with a pre-connected client (for testing)
    pub(crate) async fn add_server_with_client(
        &self,
        name: String,
        mut client: McpClient,
        config: McpServerConfig,
    ) -> Result<Vec<McpToolInfo>> {

        let tools = client
            .list_tools()
            .await
            .with_context(|| format!("Failed to list tools from MCP server '{}'", name))?;

        debug!("MCP server '{}' advertised {} tools", name, tools.len());


        self.validate_tools(&tools, &name).await?;


        {
            let servers = self.servers.read().await;
            let current_tool_count = servers.values().map(|s| s.tools.len()).sum::<usize>();
            if current_tool_count + tools.len() > self.tool_cap {
                let _ = client.close().await;
                anyhow::bail!(
                    "Adding server '{}' would exceed tool cap ({}). Current: {}, New: {}, Cap: {}",
                    name,
                    self.tool_cap,
                    current_tool_count,
                    tools.len(),
                    self.tool_cap
                );
            }
        }


        let mut servers = self.servers.write().await;
        servers.insert(
            name.clone(),
            McpServerState {
                client,
                tools: tools.clone(),
                config,
            },
        );

        info!(
            "MCP server '{}' added successfully with {} tools",
            name,
            tools.len()
        );

        Ok(tools)
    }

    async fn validate_tools(&self, tools: &[McpToolInfo], server_name: &str) -> Result<()> {
        for tool in tools {
            if self.builtin_tool_names.contains(&tool.name) {
                anyhow::bail!(
                    "MCP server '{}' tool '{}' collides with builtin tool name",
                    server_name,
                    tool.name
                );
            }
        }

        let servers = self.servers.read().await;
        for tool in tools {
            for state in servers.values() {
                for existing_tool in &state.tools {
                    if existing_tool.name == tool.name {
                        anyhow::bail!(
                            "MCP server '{}' tool '{}' collides with existing MCP tool name",
                            server_name,
                            tool.name
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Remove an MCP server from the registry
    ///
    /// Closes the client connection and removes the server from the map.
    ///
    /// # Arguments
    /// * `name` - Name of the server to remove
    ///
    /// # Returns
    /// * `Ok(())` - Server removed successfully
    /// * `Err` - Server not found or failed to close
    pub async fn remove_server(&self, name: &str) -> Result<()> {
        info!("Removing MCP server: {}", name);

        let mut servers = self.servers.write().await;
        let mut server = servers
            .remove(name)
            .with_context(|| format!("MCP server '{}' not found", name))?;


        server
            .client
            .close()
            .await
            .with_context(|| format!("Failed to close MCP server '{}'", name))?;

        info!("MCP server '{}' removed successfully", name);
        Ok(())
    }

    /// List all connected MCP servers with their tool counts
    ///
    /// # Returns
    /// Vector of (server_name, tool_count) pairs
    pub async fn list_servers(&self) -> Vec<(String, usize)> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|(name, state)| (name.clone(), state.tools.len()))
            .collect()
    }

    /// Get all tools from all connected servers
    ///
    /// # Returns
    /// Vector of (server_name, McpToolInfo) pairs for all tools across all servers
    pub async fn get_all_tools(&self) -> Vec<(String, McpToolInfo)> {
        let servers = self.servers.read().await;
        let mut all_tools = Vec::new();

        for (server_name, state) in servers.iter() {
            for tool in &state.tools {
                all_tools.push((server_name.clone(), tool.clone()));
            }
        }

        all_tools
    }

    /// Call a tool on a specific MCP server
    ///
    /// # Arguments
    /// * `server_name` - Name of the server hosting the tool
    /// * `tool_name` - Name of the tool to call
    /// * `args` - Optional JSON arguments for the tool
    ///
    /// # Returns
    /// * `Ok(McpToolCallResult)` - Tool execution result
    /// * `Err` - Server not found or tool execution failed
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: Option<serde_json::Value>,
    ) -> Result<McpToolCallResult> {
        debug!(
            "Calling MCP tool '{}' on server '{}'",
            tool_name, server_name
        );

        let mut servers = self.servers.write().await;
        let server = servers
            .get_mut(server_name)
            .with_context(|| format!("MCP server '{}' not found", server_name))?;

        server
            .client
            .call_tool(tool_name, args)
            .await
            .with_context(|| {
                format!(
                    "Failed to call tool '{}' on server '{}'",
                    tool_name, server_name
                )
            })
    }

    /// Get the number of connected servers
    #[allow(dead_code)]
    pub async fn server_count(&self) -> usize {
        let servers = self.servers.read().await;
        servers.len()
    }

    /// Get the total number of tools across all servers
    #[allow(dead_code)]
    pub async fn total_tool_count(&self) -> usize {
        let servers = self.servers.read().await;
        servers.values().map(|s| s.tools.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::client::McpClient;
    use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, RequestId};
    use crate::transport::McpTransport;

    /// Mock transport for testing
    struct MockTransport {
        responses: Mutex<VecDeque<JsonRpcResponse>>,
    }

    impl MockTransport {
        fn new(responses: Vec<JsonRpcResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl McpTransport for MockTransport {
        async fn send(&mut self, _request: &JsonRpcRequest) -> Result<()> {
            Ok(())
        }

        async fn send_notification(&mut self, _notification: &JsonRpcNotification) -> Result<()> {
            Ok(())
        }

        async fn receive(&mut self) -> Result<JsonRpcResponse> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("No more queued responses"))
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Helper to create a mock McpClient for testing
    async fn create_mock_client(tools_response: JsonRpcResponse) -> McpClient {
        let transport = MockTransport::new(vec![create_init_response(), tools_response]);
        McpClient::connect(Box::new(transport)).await.unwrap()
    }

    fn create_init_response() -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        }
    }

    fn create_tools_response(tools: Vec<serde_json::Value>) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "tools": tools
            })),
            error: None,
        }
    }

    #[tokio::test]
    async fn test_add_server() {
        let registry = McpRegistry::new(50, HashSet::new());

        let tools_json = vec![json!({
            "name": "test_tool",
            "description": "A test tool",
            "inputSchema": {"type": "object"}
        })];

        let mut client = create_mock_client(create_tools_response(tools_json)).await;

        let config = McpServerConfig {
            name: "test_server".to_string(),
            command: "test".to_string(),
            args: vec![],
            env: HashMap::new(),
        };


        let tools = registry
            .add_server_with_client("test_server".to_string(), client, config)
            .await
            .unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "test_tool");


        let servers = registry.list_servers().await;
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].0, "test_server");
        assert_eq!(servers[0].1, 1);
    }

    #[tokio::test]
    async fn test_remove_server() {
        let registry = McpRegistry::new(50, HashSet::new());

        let tools_json = vec![json!({
            "name": "test_tool",
            "description": "A test tool",
            "inputSchema": {"type": "object"}
        })];

        let mut client = create_mock_client(create_tools_response(tools_json)).await;

        let config = McpServerConfig {
            name: "test_server".to_string(),
            command: "test".to_string(),
            args: vec![],
            env: HashMap::new(),
        };

        registry
            .add_server_with_client("test_server".to_string(), client, config)
            .await
            .unwrap();


        assert_eq!(registry.list_servers().await.len(), 1);

        // Remove server
        registry.remove_server("test_server").await.unwrap();


        assert!(registry.list_servers().await.is_empty());
    }

    #[tokio::test]
    async fn test_list_servers() {
        let registry = McpRegistry::new(50, HashSet::new());


        let tools_json1 = vec![json!({
            "name": "tool1",
            "description": "First tool",
            "inputSchema": {"type": "object"}
        })];
        let client1 = create_mock_client(create_tools_response(tools_json1)).await;
        let config1 = McpServerConfig {
            name: "server1".to_string(),
            command: "test1".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        registry
            .add_server_with_client("server1".to_string(), client1, config1)
            .await
            .unwrap();


        let tools_json2 = vec![
            json!({
                "name": "tool2",
                "description": "Second tool",
                "inputSchema": {"type": "object"}
            }),
            json!({
                "name": "tool3",
                "description": "Third tool",
                "inputSchema": {"type": "object"}
            }),
        ];
        let client2 = create_mock_client(create_tools_response(tools_json2)).await;
        let config2 = McpServerConfig {
            name: "server2".to_string(),
            command: "test2".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        registry
            .add_server_with_client("server2".to_string(), client2, config2)
            .await
            .unwrap();


        let servers = registry.list_servers().await;
        assert_eq!(servers.len(), 2);

        let server_names: HashSet<_> = servers.iter().map(|(n, _)| n.as_str()).collect();
        assert!(server_names.contains("server1"));
        assert!(server_names.contains("server2"));


        let server1_tools = servers.iter().find(|(n, _)| n == "server1").unwrap().1;
        let server2_tools = servers.iter().find(|(n, _)| n == "server2").unwrap().1;
        assert_eq!(server1_tools, 1);
        assert_eq!(server2_tools, 2);
    }

    #[tokio::test]
    async fn test_builtin_collision_rejected() {
        let mut builtin_tools = HashSet::new();
        builtin_tools.insert("shell".to_string());

        let registry = McpRegistry::new(50, builtin_tools);


        let tools_json = vec![json!({
            "name": "shell",
            "description": "Shell tool",
            "inputSchema": {"type": "object"}
        })];

        let mut client = create_mock_client(create_tools_response(tools_json)).await;

        let config = McpServerConfig {
            name: "bad_server".to_string(),
            command: "test".to_string(),
            args: vec![],
            env: HashMap::new(),
        };

        let result = registry
            .add_server_with_client("bad_server".to_string(), client, config)
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("collides with builtin tool name"));
        assert!(err_msg.contains("shell"));


        assert!(registry.list_servers().await.is_empty());
    }

    #[tokio::test]
    async fn test_tool_cap_enforced() {

        let registry = McpRegistry::new(2, HashSet::new());


        let tools_json = vec![
            json!({
                "name": "tool1",
                "description": "Tool 1",
                "inputSchema": {"type": "object"}
            }),
            json!({
                "name": "tool2",
                "description": "Tool 2",
                "inputSchema": {"type": "object"}
            }),
            json!({
                "name": "tool3",
                "description": "Tool 3",
                "inputSchema": {"type": "object"}
            }),
        ];

        let mut client = create_mock_client(create_tools_response(tools_json)).await;

        let config = McpServerConfig {
            name: "big_server".to_string(),
            command: "test".to_string(),
            args: vec![],
            env: HashMap::new(),
        };

        let result = registry
            .add_server_with_client("big_server".to_string(), client, config)
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("exceed tool cap"));
        assert!(err_msg.contains("Cap: 2"));


        assert!(registry.list_servers().await.is_empty());
    }

    #[tokio::test]
    async fn test_get_all_tools() {
        let registry = McpRegistry::new(50, HashSet::new());


        let tools_json1 = vec![json!({
            "name": "tool_a",
            "description": "Tool A",
            "inputSchema": {"type": "object"}
        })];
        let client1 = create_mock_client(create_tools_response(tools_json1)).await;
        let config1 = McpServerConfig {
            name: "server_a".to_string(),
            command: "test_a".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        registry
            .add_server_with_client("server_a".to_string(), client1, config1)
            .await
            .unwrap();


        let tools_json2 = vec![
            json!({
                "name": "tool_b",
                "description": "Tool B",
                "inputSchema": {"type": "object"}
            }),
            json!({
                "name": "tool_c",
                "description": "Tool C",
                "inputSchema": {"type": "object"}
            }),
        ];
        let client2 = create_mock_client(create_tools_response(tools_json2)).await;
        let config2 = McpServerConfig {
            name: "server_b".to_string(),
            command: "test_b".to_string(),
            args: vec![],
            env: HashMap::new(),
        };
        registry
            .add_server_with_client("server_b".to_string(), client2, config2)
            .await
            .unwrap();


        let all_tools = registry.get_all_tools().await;
        assert_eq!(all_tools.len(), 3);


        let tool_names: HashSet<_> = all_tools.iter().map(|(_, t)| t.name.as_str()).collect();
        assert!(tool_names.contains("tool_a"));
        assert!(tool_names.contains("tool_b"));
        assert!(tool_names.contains("tool_c"));


        let server_a_tools: Vec<_> = all_tools.iter().filter(|(s, _)| s == "server_a").collect();
        let server_b_tools: Vec<_> = all_tools.iter().filter(|(s, _)| s == "server_b").collect();
        assert_eq!(server_a_tools.len(), 1);
        assert_eq!(server_b_tools.len(), 2);
    }

    #[tokio::test]
    async fn test_call_tool() {
        // Error path test: server not found
        let registry = McpRegistry::new(50, HashSet::new());

        let result = registry.call_tool("nonexistent", "some_tool", None).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
    }
}
