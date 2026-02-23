//! MCP-specific message types and structures

use serde::{Deserialize, Serialize};

/// Client capabilities (empty for now, extensible)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ClientCapabilities {}

/// Marker capability for tools support
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ToolsCapability {}

/// Marker capability for resources support
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ResourcesCapability {}

/// Marker capability for prompts support
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PromptsCapability {}

/// Server capabilities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
}

/// Implementation info (name and version)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

/// Initialize request parameters
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: Implementation,
}

/// Initialize response result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: Implementation,
}

/// MCP tool information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
}

/// MCP tool call parameters
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,
}

/// MCP tool call result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResult {
    pub content: Vec<McpContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isError")]
    pub is_error: Option<bool>,
}

/// MCP content (text type)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// MCP resource
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// MCP resource content
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// MCP prompt argument
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// MCP prompt
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpPrompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<McpPromptArgument>>,
}

/// MCP prompt message
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptMessage {
    pub role: String,
    pub content: McpContent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_implementation_roundtrip() {
        let impl_info = Implementation {
            name: "ZeroClaw".to_string(),
            version: "0.1.0".to_string(),
        };

        let serialized = serde_json::to_string(&impl_info).unwrap();
        let deserialized: Implementation = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, impl_info);
    }

    #[test]
    fn test_initialize_params_roundtrip() {
        let params = InitializeParams {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "ZeroClaw".to_string(),
                version: "0.1.0".to_string(),
            },
        };

        let serialized = serde_json::to_string(&params).unwrap();
        let deserialized: InitializeParams = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, params);
        assert_eq!(deserialized.protocol_version, "2024-11-05");
    }

    #[test]
    fn test_initialize_result_roundtrip() {
        let result = InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability::default()),
                resources: None,
                prompts: Some(PromptsCapability::default()),
            },
            server_info: Implementation {
                name: "ZeroClaw MCP Server".to_string(),
                version: "0.1.0".to_string(),
            },
        };

        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: InitializeResult = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, result);
        assert!(deserialized.capabilities.tools.is_some());
        assert!(deserialized.capabilities.resources.is_none());
        assert!(deserialized.capabilities.prompts.is_some());
    }

    #[test]
    fn test_mcp_tool_info_roundtrip() {
        let tool_info = McpToolInfo {
            name: "file_read".to_string(),
            description: Some("Read a file from the filesystem".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"]
            }),
        };

        let serialized = serde_json::to_string(&tool_info).unwrap();
        let deserialized: McpToolInfo = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, tool_info);
        assert!(deserialized.description.is_some());
    }

    #[test]
    fn test_mcp_tool_call_params_roundtrip() {
        let params = McpToolCallParams {
            name: "shell".to_string(),
            arguments: Some(json!({"command": "ls -la"})),
        };

        let serialized = serde_json::to_string(&params).unwrap();
        let deserialized: McpToolCallParams = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, params);
    }

    #[test]
    fn test_mcp_tool_call_result_roundtrip() {
        let result = McpToolCallResult {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: Some("file1.txt\nfile2.txt".to_string()),
            }],
            is_error: Some(false),
        };

        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: McpToolCallResult = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, result);
        assert_eq!(deserialized.content.len(), 1);
    }

    #[test]
    fn test_mcp_content_roundtrip() {
        let content = McpContent {
            content_type: "text".to_string(),
            text: Some("Hello, MCP!".to_string()),
        };

        let serialized = serde_json::to_string(&content).unwrap();
        let deserialized: McpContent = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, content);
    }

    #[test]
    fn test_mcp_resource_roundtrip() {
        let resource = McpResource {
            uri: "file:///etc/config.toml".to_string(),
            name: "config.toml".to_string(),
            description: Some("Configuration file".to_string()),
            mime_type: Some("text/plain".to_string()),
        };

        let serialized = serde_json::to_string(&resource).unwrap();
        let deserialized: McpResource = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, resource);
    }

    #[test]
    fn test_mcp_resource_content_roundtrip() {
        let content = McpResourceContent {
            uri: "file:///tmp/data.json".to_string(),
            mime_type: Some("application/json".to_string()),
            text: Some(r#"{"key": "value"}"#.to_string()),
        };

        let serialized = serde_json::to_string(&content).unwrap();
        let deserialized: McpResourceContent = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, content);
    }

    #[test]
    fn test_mcp_prompt_roundtrip() {
        let prompt = McpPrompt {
            name: "code_review".to_string(),
            description: Some("Review code for best practices".to_string()),
            arguments: Some(vec![McpPromptArgument {
                name: "code".to_string(),
                description: Some("The code to review".to_string()),
                required: Some(true),
            }]),
        };

        let serialized = serde_json::to_string(&prompt).unwrap();
        let deserialized: McpPrompt = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, prompt);
        assert!(deserialized.arguments.is_some());
    }

    #[test]
    fn test_mcp_prompt_message_roundtrip() {
        let message = McpPromptMessage {
            role: "user".to_string(),
            content: McpContent {
                content_type: "text".to_string(),
                text: Some("Review this code".to_string()),
            },
        };

        let serialized = serde_json::to_string(&message).unwrap();
        let deserialized: McpPromptMessage = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, message);
    }

    #[test]
    fn test_server_capabilities_serialization() {
        let capabilities = ServerCapabilities {
            tools: Some(ToolsCapability::default()),
            resources: Some(ResourcesCapability::default()),
            prompts: None,
        };

        let serialized = serde_json::to_string(&capabilities).unwrap();

        // Verify camelCase keys
        assert!(serialized.contains("tools"));
        assert!(serialized.contains("resources"));
        assert!(!serialized.contains("prompts"));
    }
}
