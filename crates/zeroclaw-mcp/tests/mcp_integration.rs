//! MCP Integration Tests
//!
//! Tests for zeroclaw-mcp crate's registry, config, and client components.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;
use tempfile::TempDir;
use zeroclaw_mcp::client::McpClient;
use zeroclaw_mcp::config::{parse_mcp_config, McpServerConfig};
use zeroclaw_mcp::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, RequestId};
use zeroclaw_mcp::registry::McpRegistry;
use zeroclaw_mcp::transport::McpTransport;

/// Mock transport for testing registry operations
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
    async fn send(&mut self, _request: &JsonRpcRequest) -> anyhow::Result<()> {
        Ok(())
    }

    async fn send_notification(
        &mut self,
        _notification: &JsonRpcNotification,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn receive(&mut self) -> anyhow::Result<JsonRpcResponse> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("No more queued responses"))
    }

    async fn close(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
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
        result: Some(json!({"tools": tools})),
        error: None,
    }
}

async fn create_mock_client(tools_response: JsonRpcResponse) -> McpClient {
    let transport = MockTransport::new(vec![create_init_response(), tools_response]);
    McpClient::connect(Box::new(transport)).await.unwrap()
}

#[tokio::test]
async fn test_registry_creation_with_tool_cap() {
    let builtin_tools = HashSet::new();
    let registry = McpRegistry::new(10, builtin_tools);

    let tools_json = vec![json!({
        "name": "test_tool",
        "description": "A test tool",
        "inputSchema": {"type": "object"}
    })];

    let client = create_mock_client(create_tools_response(tools_json)).await;
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
}

#[tokio::test]
async fn test_tool_cap_enforced() {
    let registry = McpRegistry::new(2, HashSet::new());

    let tools_json = vec![
        json!({"name": "tool1", "description": "Tool 1", "inputSchema": {"type": "object"}}),
        json!({"name": "tool2", "description": "Tool 2", "inputSchema": {"type": "object"}}),
        json!({"name": "tool3", "description": "Tool 3", "inputSchema": {"type": "object"}}),
    ];

    let client = create_mock_client(create_tools_response(tools_json)).await;
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
}

#[tokio::test]
async fn test_tool_name_collision_rejected() {
    let mut builtin_tools = HashSet::new();
    builtin_tools.insert("shell".to_string());

    let registry = McpRegistry::new(50, builtin_tools);

    let tools_json = vec![json!({
        "name": "shell",
        "description": "Shell tool",
        "inputSchema": {"type": "object"}
    })];

    let client = create_mock_client(create_tools_response(tools_json)).await;
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
}

#[tokio::test]
async fn test_registry_list_servers() {
    let registry = McpRegistry::new(50, HashSet::new());

    let tools_json1 =
        vec![json!({"name": "tool1", "description": "Tool 1", "inputSchema": {"type": "object"}})];
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
        json!({"name": "tool2", "description": "Tool 2", "inputSchema": {"type": "object"}}),
        json!({"name": "tool3", "description": "Tool 3", "inputSchema": {"type": "object"}}),
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

#[test]
fn test_config_parsing_from_json() {
    let json = r#"{
        "mcpServers": {
            "filesystem": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                "env": {"NODE_ENV": "production"}
            },
            "git": {
                "command": "node",
                "args": ["/path/to/git-server.js"],
                "env": {"DEBUG": "true"}
            }
        }
    }"#;

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join(".mcp.json");
    std::fs::write(&config_path, json).unwrap();

    let configs = parse_mcp_config(&config_path).expect("Failed to parse config");

    assert_eq!(configs.len(), 2);

    let fs_config = configs.iter().find(|c| c.name == "filesystem").unwrap();
    assert_eq!(fs_config.command, "npx");
    assert_eq!(
        fs_config.args,
        vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    );
    assert_eq!(
        fs_config.env.get("NODE_ENV"),
        Some(&"production".to_string())
    );

    let git_config = configs.iter().find(|c| c.name == "git").unwrap();
    assert_eq!(git_config.command, "node");
}
