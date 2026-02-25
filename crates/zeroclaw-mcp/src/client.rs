//! MCP client implementation for tool operations

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context, Result};
use tracing::{debug, warn};

use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, RequestId};
use crate::transport::McpTransport;
use crate::types::{
    ClientCapabilities, Implementation, InitializeParams, InitializeResult, McpPrompt,
    McpPromptMessage, McpResource, McpResourceContent, McpToolCallParams, McpToolCallResult,
    McpToolInfo, ServerCapabilities,
};

/// Maximum number of pagination pages to fetch before stopping (safety guard).
const MAX_PAGES: usize = 100;

pub struct McpClient {
    transport: Box<dyn McpTransport>,
    next_id: AtomicI64,
    server_capabilities: Option<ServerCapabilities>,
}

impl McpClient {
    pub async fn connect(mut transport: Box<dyn McpTransport>) -> Result<Self> {
        debug!("Starting MCP client handshake");

        let init_params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "zeroclaw".to_string(),
                version: "0.1.0".to_string(),
            },
        };

        let init_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            method: "initialize".to_string(),
            params: Some(serde_json::to_value(&init_params)?),
        };

        transport
            .send(&init_request)
            .await
            .context("Failed to send initialize request")?;

        let init_response = transport
            .receive()
            .await
            .context("Failed to receive initialize response")?;

        if let Some(error) = init_response.error {
            anyhow::bail!("Initialize failed: {}", error.message);
        }

        let result_value = init_response
            .result
            .context("Initialize response missing result")?;
        let init_result: InitializeResult =
            serde_json::from_value(result_value).context("Failed to parse InitializeResult")?;

        debug!(
            "MCP server initialized: protocol={}, server={}",
            init_result.protocol_version, init_result.server_info.name
        );

        let initialized_notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
        };

        transport
            .send_notification(&initialized_notification)
            .await
            .context("Failed to send initialized notification")?;

        debug!("MCP client handshake complete");

        Ok(Self {
            transport,
            next_id: AtomicI64::new(2),
            server_capabilities: Some(init_result.capabilities),
        })
    }

    fn next_request_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolInfo>> {
        debug!("Requesting tools list");

        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        for _page in 0..MAX_PAGES {
            let params = cursor.as_ref().map(|c| serde_json::json!({"cursor": c}));

            let request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: RequestId::Number(self.next_request_id()),
                method: "tools/list".to_string(),
                params,
            };

            self.transport
                .send(&request)
                .await
                .context("Failed to send tools/list request")?;

            let response = self
                .transport
                .receive()
                .await
                .context("Failed to receive tools/list response")?;

            if let Some(error) = response.error {
                anyhow::bail!("tools/list failed: {}", error.message);
            }

            let result_value = response
                .result
                .context("tools/list response missing result")?;

            let tools_value = result_value
                .get("tools")
                .context("tools/list result missing 'tools' field")?;

            let tools: Vec<McpToolInfo> = serde_json::from_value(tools_value.clone())
                .context("Failed to parse tools list")?;

            all_tools.extend(tools);

            cursor = result_value
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if cursor.is_none() {
                break;
            }
        }

        if cursor.is_some() {
            warn!(
                "Pagination limit reached (MAX_PAGES={}), results may be incomplete",
                MAX_PAGES
            );
        }

        debug!("Retrieved {} tools", all_tools.len());
        Ok(all_tools)
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        args: Option<serde_json::Value>,
    ) -> Result<McpToolCallResult> {
        debug!("Calling tool: {}", name);

        let params = McpToolCallParams {
            name: name.to_string(),
            arguments: args,
        };

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(self.next_request_id()),
            method: "tools/call".to_string(),
            params: Some(serde_json::to_value(&params)?),
        };

        self.transport
            .send(&request)
            .await
            .context("Failed to send tools/call request")?;

        let response = self
            .transport
            .receive()
            .await
            .context("Failed to receive tools/call response")?;

        if let Some(error) = response.error {
            anyhow::bail!("tools/call failed: {}", error.message);
        }

        let result_value = response
            .result
            .context("tools/call response missing result")?;

        let result: McpToolCallResult =
            serde_json::from_value(result_value).context("Failed to parse McpToolCallResult")?;

        if result.is_error == Some(true) {
            let error_msg = result
                .content
                .first()
                .and_then(|c| c.text.as_ref())
                .map(|t| t.as_str())
                .unwrap_or("Unknown tool error");
            anyhow::bail!("Tool execution error: {}", error_msg);
        }

        Ok(result)
    }

    /// List available resources
    pub async fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        debug!("Requesting resources list");
        let mut all_resources = Vec::new();
        let mut cursor: Option<String> = None;
        for _page in 0..MAX_PAGES {
            let params = cursor.as_ref().map(|c| serde_json::json!({"cursor": c}));
            let request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: RequestId::Number(self.next_request_id()),
                method: "resources/list".to_string(),
                params,
            };
            self.transport
                .send(&request)
                .await
                .context("Failed to send resources/list request")?;
            let response = self
                .transport
                .receive()
                .await
                .context("Failed to receive resources/list response")?;
            if let Some(error) = response.error {
                anyhow::bail!("resources/list failed: {}", error.message);
            }
            let result_value = response
                .result
                .context("resources/list response missing result")?;
            let resources_value = result_value
                .get("resources")
                .context("resources/list result missing 'resources' field")?;
            let resources: Vec<McpResource> = serde_json::from_value(resources_value.clone())
                .context("Failed to parse resources list")?;
            all_resources.extend(resources);
            cursor = result_value
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }
        if cursor.is_some() {
            warn!(
                "Pagination limit reached (MAX_PAGES={}), results may be incomplete",
                MAX_PAGES
            );
        }
        debug!("Retrieved {} resources", all_resources.len());
        Ok(all_resources)
    }

    /// Read a resource by URI
    pub async fn read_resource(&mut self, uri: &str) -> Result<Vec<McpResourceContent>> {
        debug!("Reading resource: {}", uri);

        let params = serde_json::json!({
            "uri": uri
        });

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(self.next_request_id()),
            method: "resources/read".to_string(),
            params: Some(params),
        };

        self.transport
            .send(&request)
            .await
            .context("Failed to send resources/read request")?;

        let response = self
            .transport
            .receive()
            .await
            .context("Failed to receive resources/read response")?;

        if let Some(error) = response.error {
            anyhow::bail!("resources/read failed: {}", error.message);
        }

        let result_value = response
            .result
            .context("resources/read response missing result")?;

        let contents_value = result_value
            .get("contents")
            .context("resources/read result missing 'contents' field")?;

        let contents: Vec<McpResourceContent> = serde_json::from_value(contents_value.clone())
            .context("Failed to parse resource contents")?;

        debug!("Read {} content items", contents.len());
        Ok(contents)
    }

    /// List available prompts
    pub async fn list_prompts(&mut self) -> Result<Vec<McpPrompt>> {
        debug!("Requesting prompts list");
        let mut all_prompts = Vec::new();
        let mut cursor: Option<String> = None;
        for _page in 0..MAX_PAGES {
            let params = cursor.as_ref().map(|c| serde_json::json!({"cursor": c}));
            let request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: RequestId::Number(self.next_request_id()),
                method: "prompts/list".to_string(),
                params,
            };
            self.transport
                .send(&request)
                .await
                .context("Failed to send prompts/list request")?;
            let response = self
                .transport
                .receive()
                .await
                .context("Failed to receive prompts/list response")?;
            if let Some(error) = response.error {
                anyhow::bail!("prompts/list failed: {}", error.message);
            }
            let result_value = response
                .result
                .context("prompts/list response missing result")?;
            let prompts_value = result_value
                .get("prompts")
                .context("prompts/list result missing 'prompts' field")?;
            let prompts: Vec<McpPrompt> = serde_json::from_value(prompts_value.clone())
                .context("Failed to parse prompts list")?;
            all_prompts.extend(prompts);
            cursor = result_value
                .get("nextCursor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
        }
        if cursor.is_some() {
            warn!(
                "Pagination limit reached (MAX_PAGES={}), results may be incomplete",
                MAX_PAGES
            );
        }
        debug!("Retrieved {} prompts", all_prompts.len());
        Ok(all_prompts)
    }

    /// Get a prompt by name with optional arguments
    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: Option<HashMap<String, String>>,
    ) -> Result<Vec<McpPromptMessage>> {
        debug!("Getting prompt: {}", name);

        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(self.next_request_id()),
            method: "prompts/get".to_string(),
            params: Some(params),
        };

        self.transport
            .send(&request)
            .await
            .context("Failed to send prompts/get request")?;

        let response = self
            .transport
            .receive()
            .await
            .context("Failed to receive prompts/get response")?;

        if let Some(error) = response.error {
            anyhow::bail!("prompts/get failed: {}", error.message);
        }

        let result_value = response
            .result
            .context("prompts/get response missing result")?;

        let messages_value = result_value
            .get("messages")
            .context("prompts/get result missing 'messages' field")?;

        let messages: Vec<McpPromptMessage> = serde_json::from_value(messages_value.clone())
            .context("Failed to parse prompt messages")?;

        debug!("Retrieved {} prompt messages", messages.len());
        Ok(messages)
    }

    /// Close the MCP connection
    pub async fn close(&mut self) -> Result<()> {
        debug!("Closing MCP client connection");
        self.transport.close().await
    }

    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.server_capabilities.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::jsonrpc::JsonRpcResponse;

    struct MockTransport {
        responses: VecDeque<JsonRpcResponse>,
        sent_requests: std::sync::Mutex<Vec<JsonRpcRequest>>,
        sent_notifications: std::sync::Mutex<Vec<JsonRpcNotification>>,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                responses: VecDeque::new(),
                sent_requests: std::sync::Mutex::new(Vec::new()),
                sent_notifications: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn queue_response(&mut self, response: JsonRpcResponse) {
            self.responses.push_back(response);
        }
    }

    #[async_trait]
    impl McpTransport for MockTransport {
        async fn send(&mut self, request: &JsonRpcRequest) -> Result<()> {
            self.sent_requests.lock().unwrap().push(request.clone());
            Ok(())
        }

        async fn send_notification(&mut self, notification: &JsonRpcNotification) -> Result<()> {
            self.sent_notifications
                .lock()
                .unwrap()
                .push(notification.clone());
            Ok(())
        }

        async fn receive(&mut self) -> Result<JsonRpcResponse> {
            self.responses
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("No more queued responses"))
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_connect_handshake() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        let client = McpClient::connect(Box::new(mock)).await.unwrap();

        assert!(client.server_capabilities.is_some());
        assert!(client.server_capabilities.as_ref().unwrap().tools.is_some());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "tools": [
                    {
                        "name": "file_read",
                        "description": "Read a file",
                        "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}
                    },
                    {
                        "name": "shell",
                        "description": null,
                        "inputSchema": {"type": "object", "properties": {"command": {"type": "string"}}}
                    }
                ]
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let tools = client.list_tools().await.unwrap();

        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "file_read");
        assert_eq!(tools[0].description, Some("Read a file".to_string()));
        assert_eq!(tools[1].name, "shell");
        assert!(tools[1].description.is_none());
    }

    #[tokio::test]
    async fn test_call_tool() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "content": [
                    {
                        "type": "text",
                        "text": "file1.txt\nfile2.txt"
                    }
                ],
                "isError": false
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let result = client
            .call_tool("shell", Some(json!({"command": "ls"})))
            .await
            .unwrap();

        assert_eq!(result.content.len(), 1);
        assert_eq!(result.content[0].content_type, "text");
        assert_eq!(
            result.content[0].text,
            Some("file1.txt\nfile2.txt".to_string())
        );
        assert_eq!(result.is_error, Some(false));
    }

    #[tokio::test]
    async fn test_list_resources() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"resources": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "resources": [
                    {
                        "uri": "file:///etc/config.toml",
                        "name": "config.toml",
                        "description": "Configuration file",
                        "mimeType": "text/plain"
                    }
                ]
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let resources = client.list_resources().await.unwrap();

        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].uri, "file:///etc/config.toml");
        assert_eq!(resources[0].name, "config.toml");
    }

    #[tokio::test]
    async fn test_read_resource() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"resources": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "contents": [
                    {
                        "uri": "file:///etc/config.toml",
                        "mimeType": "text/plain",
                        "text": "key = value"
                    }
                ]
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let contents = client
            .read_resource("file:///etc/config.toml")
            .await
            .unwrap();

        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].uri, "file:///etc/config.toml");
        assert_eq!(contents[0].text, Some("key = value".to_string()));
    }

    #[tokio::test]
    async fn test_list_prompts() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"prompts": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "prompts": [
                    {
                        "name": "code_review",
                        "description": "Review code for best practices",
                        "arguments": [
                            {
                                "name": "code",
                                "description": "The code to review",
                                "required": true
                            }
                        ]
                    }
                ]
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let prompts = client.list_prompts().await.unwrap();

        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].name, "code_review");
        assert!(prompts[0].arguments.is_some());
    }

    #[tokio::test]
    async fn test_get_prompt() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"prompts": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "Review this code"
                        }
                    }
                ]
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let mut args = HashMap::new();
        args.insert("code".to_string(), "fn main() {}".to_string());
        let messages = client.get_prompt("code_review", Some(args)).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(
            messages[0].content.text,
            Some("Review this code".to_string())
        );
    }

    #[tokio::test]
    async fn test_close() {
        let mut mock = MockTransport::new();

        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();

        let result = client.close().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_tools_pagination() {
        let mut mock = MockTransport::new();

        // Handshake response
        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        // Page 1: one tool + nextCursor
        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            result: Some(json!({
                "tools": [
                    {
                        "name": "tool_a",
                        "description": "First tool",
                        "inputSchema": {"type": "object", "properties": {}}
                    }
                ],
                "nextCursor": "cursor_page2"
            })),
            error: None,
        });

        // Page 2: one tool, no nextCursor (last page)
        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(3),
            result: Some(json!({
                "tools": [
                    {
                        "name": "tool_b",
                        "description": "Second tool",
                        "inputSchema": {"type": "object", "properties": {}}
                    }
                ]
            })),
            error: None,
        });

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();
        let tools = client.list_tools().await.unwrap();

        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "tool_a");
        assert_eq!(tools[1].name, "tool_b");

        // The fact that we got 2 tools from 2 pages proves pagination worked
    }

    #[tokio::test]
    async fn test_list_tools_max_pages_guard() {
        let mut mock = MockTransport::new();

        // Handshake response
        mock.queue_response(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "TestServer", "version": "1.0.0"}
            })),
            error: None,
        });

        // Queue MAX_PAGES responses, each with nextCursor
        for i in 0..super::MAX_PAGES {
            let id = (i as i64) + 2;
            let cursor_val = format!("cursor_{}", i + 1);
            mock.queue_response(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: RequestId::Number(id),
                result: Some(json!({
                    "tools": [
                        {
                            "name": format!("tool_{}", i),
                            "description": null,
                            "inputSchema": {"type": "object", "properties": {}}
                        }
                    ],
                    "nextCursor": cursor_val
                })),
                error: None,
            });
        }

        let mut client = McpClient::connect(Box::new(mock)).await.unwrap();
        let tools = client.list_tools().await.unwrap();

        // Should have collected one tool per page
        assert_eq!(tools.len(), super::MAX_PAGES);
        assert_eq!(tools[0].name, "tool_0");
        assert_eq!(
            tools[super::MAX_PAGES - 1].name,
            format!("tool_{}", super::MAX_PAGES - 1)
        );
    }
}
