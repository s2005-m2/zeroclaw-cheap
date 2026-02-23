//! MCP transport layer implementations
//!
//! This module provides transport implementations for MCP JSON-RPC communication.
//! Currently supports stdio-based transport using newline-delimited JSON framing.

use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tracing::{debug, error, info};

use crate::jsonrpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// MCP transport trait for async JSON-RPC communication
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a JSON-RPC request
    async fn send(&mut self, request: &JsonRpcRequest) -> Result<()>;

    /// Send a JSON-RPC notification (no response expected)
    async fn send_notification(&mut self, notification: &JsonRpcNotification) -> Result<()>;

    /// Receive a JSON-RPC response
    async fn receive(&mut self) -> Result<JsonRpcResponse>;

    /// Close the transport and cleanup resources
    async fn close(&mut self) -> Result<()>;
}

/// Stdio-based MCP transport that spawns a child process
///
/// Uses newline-delimited JSON framing:
/// - Each message is serialized to JSON followed by a newline
/// - Reading stops at each newline boundary
#[derive(Debug)]
pub struct StdioTransport {
    child: Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Option<BufReader<tokio::process::ChildStdout>>,
}

impl StdioTransport {
    /// Create a new stdio transport by spawning a child process
    ///
    /// # Arguments
    /// * `command` - The command to execute (e.g., "node", "python")
    /// * `args` - Command-line arguments to pass to the process
    /// * `env` - Environment variables to set for the process
    ///
    /// # Returns
    /// * `Ok(Self)` - Transport successfully created
    /// * `Err` - Failed to spawn the process
    pub async fn new(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self> {
        info!("Spawning MCP server: {} with {} args", command, args.len());

        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context(format!("Failed to spawn command: {}", command))?;

        let stdin = child.stdin.take().context("Failed to open stdin")?;
        let stdout = child.stdout.take().context("Failed to open stdout")?;
        let stdout_reader = BufReader::new(stdout);

        debug!("MCP server spawned successfully");

        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: Some(stdout_reader),
        })
    }

    /// Serialize a value to JSON and write it with a newline
    async fn write_json(&mut self, value: &Value) -> Result<()> {
        let json = serde_json::to_string(value).context("Failed to serialize JSON")?;
        let line = format!("{}\n", json);

        let stdin = self
            .stdin
            .as_mut()
            .context("Stdin already closed or not available")?;

        stdin
            .write_all(line.as_bytes())
            .await
            .context("Failed to write to stdin")?;
        stdin.flush().await.context("Failed to flush stdin")?;

        debug!("Sent: {}", json);
        Ok(())
    }

    /// Read a line from stdout and deserialize as JSON
    async fn read_json(&mut self) -> Result<Value> {
        let stdout = self
            .stdout
            .as_mut()
            .context("Stdout already closed or not available")?;

        let mut line = String::new();
        stdout
            .read_line(&mut line)
            .await
            .context("Failed to read from stdout")?;

        let line = line.trim();
        if line.is_empty() {
            anyhow::bail!("Received empty line from MCP server");
        }

        debug!("Received: {}", line);

        let value: Value =
            serde_json::from_str(line).context(format!("Failed to parse JSON: {}", line))?;

        Ok(value)
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send(&mut self, request: &JsonRpcRequest) -> Result<()> {
        debug!("Sending JSON-RPC request: method={}", request.method);
        let value = serde_json::to_value(request).context("Failed to serialize request")?;
        self.write_json(&value).await
    }

    async fn send_notification(&mut self, notification: &JsonRpcNotification) -> Result<()> {
        debug!(
            "Sending JSON-RPC notification: method={}",
            notification.method
        );
        let value =
            serde_json::to_value(notification).context("Failed to serialize notification")?;
        self.write_json(&value).await
    }

    async fn receive(&mut self) -> Result<JsonRpcResponse> {
        debug!("Waiting for JSON-RPC response");
        let value = self.read_json().await?;
        let response: JsonRpcResponse =
            serde_json::from_value(value).context("Failed to deserialize response")?;
        Ok(response)
    }

    async fn close(&mut self) -> Result<()> {
        info!("Closing MCP transport");

        // Close stdin to signal EOF to child process
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.shutdown().await;
        }

        // Close stdout reader
        self.stdout = None;

        // Kill the child process if still running
        match self.child.kill().await {
            Ok(_) => {
                info!("MCP server process killed successfully");
            }
            Err(e) => {
                error!("Failed to kill MCP server process: {}", e);
            }
        }

        // Wait for process to exit
        match self.child.wait().await {
            Ok(status) => {
                info!("MCP server exited with status: {}", status);
            }
            Err(e) => {
                error!("Failed to wait for MCP server: {}", e);
            }
        }

        Ok(())
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Ensure cleanup on drop
        if self.child.try_wait().unwrap().is_some() {
            // Process already exited, nothing to do
        } else {
            let _ = self.child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_stdio_transport_new_with_cat() {
        // Test spawning a simple process that stays alive
        // Using `cat` on Unix-like systems or a simple echo approach
        #[cfg(unix)]
        {
            let mut env = HashMap::new();
            env.insert("TEST_VAR".to_string(), "test_value".to_string());

            let result = StdioTransport::new("cat", &[], &env).await;
            assert!(result.is_ok(), "Failed to spawn cat process: {:?}", result);

            let mut transport = result.unwrap();
            let close_result = transport.close().await;
            assert!(close_result.is_ok());
        }

        // On Windows, we can use a simple PowerShell command
        #[cfg(windows)]
        {
            let mut env = HashMap::new();
            env.insert("TEST_VAR".to_string(), "test_value".to_string());

            // Use PowerShell to create a simple echo-like process
            let result = StdioTransport::new(
                "powershell",
                &[
                    "-Command".to_string(),
                    "while ($true) { Start-Sleep -Milliseconds 100 }".to_string(),
                ],
                &env,
            )
            .await;
            assert!(
                result.is_ok(),
                "Failed to spawn powershell process"
            );

            let mut transport = result.unwrap();
            let close_result = transport.close().await;
            assert!(close_result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_stdio_transport_serialization() {
        // Test that we can serialize requests properly
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: crate::jsonrpc::RequestId::Number(1),
            method: "initialize".to_string(),
            params: Some(json!({"protocolVersion": "2024-11-05"})),
        };

        let serialized = serde_json::to_value(&request).unwrap();
        assert_eq!(serialized["jsonrpc"], "2.0");
        assert_eq!(serialized["id"], 1);
        assert_eq!(serialized["method"], "initialize");
        assert!(serialized["params"].is_object());
    }

    #[tokio::test]
    async fn test_stdio_transport_notification_serialization() {
        // Test that we can serialize notifications properly
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: "notifications/initialized".to_string(),
            params: None,
        };

        let serialized = serde_json::to_value(&notification).unwrap();
        assert_eq!(serialized["jsonrpc"], "2.0");
        assert_eq!(serialized["method"], "notifications/initialized");
        assert!(serialized["params"].is_null());
    }

    #[tokio::test]
    async fn test_stdio_transport_response_deserialization() {
        // Test that we can deserialize responses properly
        let response_json = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"protocolVersion": "2024-11-05"}
        });

        let response: JsonRpcResponse = serde_json::from_value(response_json).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(response.id, crate::jsonrpc::RequestId::Number(1));
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_stdio_transport_error_deserialization() {
        // Test that we can deserialize error responses properly
        let response_json = json!({
            "jsonrpc": "2.0",
            "id": "req-123",
            "error": {
                "code": -32600,
                "message": "Invalid Request",
                "data": {"details": "missing params"}
            }
        });

        let response: JsonRpcResponse = serde_json::from_value(response_json).unwrap();
        assert_eq!(response.jsonrpc, "2.0");
        assert_eq!(
            response.id,
            crate::jsonrpc::RequestId::String("req-123".to_string())
        );
        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_stdio_transport_send_receive_echo() {
        // Test send/receive roundtrip with a process that echoes input
        // This is a more comprehensive integration test
        use std::io::Write;

        // Create a simple test script that echoes JSON
        let temp_dir = std::env::temp_dir();
        let script_path = temp_dir.join("zeroclaw_mcp_test_echo.sh");

        // Write a simple echo script
        let script_content = r#"#!/bin/bash
while IFS= read -r line; do
    echo "$line"
done
"#;

        let mut file = std::fs::File::create(&script_path).unwrap();
        file.write_all(script_content.as_bytes()).unwrap();

        // Make it executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script_path, perms).unwrap();
        }

        let env = HashMap::new();
        let result = StdioTransport::new(script_path.to_str().unwrap(), &[], &env).await;

        assert!(result.is_ok(), "Failed to spawn echo script: {:?}", result);

        let mut transport = result.unwrap();

        // Send a request
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: crate::jsonrpc::RequestId::Number(42),
            method: "test/method".to_string(),
            params: Some(json!({"test": "data"})),
        };

        let send_result = transport.send(&request).await;
        assert!(
            send_result.is_ok(),
            "Failed to send request: {:?}",
            send_result
        );

        // Receive the echoed response
        let receive_result = transport.receive().await;
        assert!(
            receive_result.is_ok(),
            "Failed to receive response: {:?}",
            receive_result
        );

        let response = receive_result.unwrap();
        assert_eq!(response.id, crate::jsonrpc::RequestId::Number(42));
        assert_eq!(response.jsonrpc, "2.0");

        // Cleanup
        let close_result = transport.close().await;
        assert!(close_result.is_ok());

        // Remove temp script
        let _ = std::fs::remove_file(&script_path);
    }

    #[tokio::test]
    async fn test_stdio_transport_close_kills_process() {
        #[cfg(unix)]
        {
            // Spawn a long-running process
            let result = StdioTransport::new("sleep", &["60".to_string()], &HashMap::new()).await;
            assert!(result.is_ok());

            let mut transport = result.unwrap();

            // Close should kill the process
            let close_result = transport.close().await;
            assert!(close_result.is_ok());

            // Verify process is dead (try_wait should return Some)
            // This is implicitly tested by close() waiting for the process
        }

        #[cfg(windows)]
        {
            // Spawn a long-running PowerShell process
            let result = StdioTransport::new(
                "powershell",
                &[
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 60".to_string(),
                ],
                &HashMap::new(),
            )
            .await;
            assert!(result.is_ok());

            let mut transport = result.unwrap();

            // Close should kill the process
            let close_result = transport.close().await;
            assert!(close_result.is_ok());
        }
    }

    #[tokio::test]
    async fn test_newline_delimited_framing() {
        // Test that JSON is properly framed with newlines
        let value = json!({"jsonrpc": "2.0", "id": 1, "method": "test"});
        let json_str = serde_json::to_string(&value).unwrap();
        let framed = format!("{}\n", json_str);

        // Verify framing
        assert!(framed.ends_with('\n'));
        assert_eq!(framed.matches('\n').count(), 1);

        // Verify deserialization after trimming
        let trimmed = framed.trim();
        let parsed: Value = serde_json::from_str(trimmed).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
    }
}
