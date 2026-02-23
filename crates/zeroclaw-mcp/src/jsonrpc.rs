//! JSON-RPC 2.0 protocol types for MCP

use serde::{Deserialize, Serialize};

/// JSON-RPC request ID (number or string)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

/// JSON-RPC error object
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    #[serde(rename = "jsonrpc", default = "jsonrpc_version")]
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response (success or error)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    #[serde(rename = "jsonrpc", default = "jsonrpc_version")]
    pub jsonrpc: String,
    pub id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 notification (no id field)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcNotification {
    #[serde(rename = "jsonrpc", default = "jsonrpc_version")]
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

fn jsonrpc_version() -> String {
    "2.0".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_request_id_number() {
        let id = RequestId::Number(42);
        let serialized = serde_json::to_string(&id).unwrap();
        assert_eq!(serialized, "42");

        let deserialized: RequestId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, id);
    }

    #[test]
    fn test_request_id_string() {
        let id = RequestId::String("test-id".to_string());
        let serialized = serde_json::to_string(&id).unwrap();
        assert_eq!(serialized, "\"test-id\"");

        let deserialized: RequestId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, id);
    }

    #[test]
    fn test_jsonrpc_request_roundtrip() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            method: "initialize".to_string(),
            params: Some(json!({"protocolVersion": "2024-11-05"})),
        };

        let serialized = serde_json::to_string(&request).unwrap();
        let deserialized: JsonRpcRequest = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, request);
        assert_eq!(deserialized.jsonrpc, "2.0");
    }

    #[test]
    fn test_jsonrpc_response_success_roundtrip() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            result: Some(json!({"protocolVersion": "2024-11-05"})),
            error: None,
        };

        let serialized = serde_json::to_string(&response).unwrap();
        let deserialized: JsonRpcResponse = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, response);
        assert!(deserialized.result.is_some());
        assert!(deserialized.error.is_none());
    }

    #[test]
    fn test_jsonrpc_response_error_roundtrip() {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: RequestId::String("req-123".to_string()),
            result: None,
            error: Some(JsonRpcError {
                code: -32600,
                message: "Invalid Request".to_string(),
                data: Some(json!({"details": "missing params"})),
            }),
        };

        let serialized = serde_json::to_string(&response).unwrap();
        let deserialized: JsonRpcResponse = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, response);
        assert!(deserialized.result.is_none());
        assert!(deserialized.error.is_some());
    }

    #[test]
    fn test_jsonrpc_notification_roundtrip() {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
        };

        let serialized = serde_json::to_string(&notification).unwrap();
        let deserialized: JsonRpcNotification = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, notification);
        assert_eq!(deserialized.jsonrpc, "2.0");
    }

    #[test]
    fn test_jsonrpc_error_roundtrip() {
        let error = JsonRpcError {
            code: -32601,
            message: "Method not found".to_string(),
            data: None,
        };

        let serialized = serde_json::to_string(&error).unwrap();
        let deserialized: JsonRpcError = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, error);
    }

    #[test]
    fn test_request_with_string_id() {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::String("abc-123".to_string()),
            method: "tools/list".to_string(),
            params: None,
        };

        let serialized = serde_json::to_string(&request).unwrap();
        let deserialized: JsonRpcRequest = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized, request);
    }
}
