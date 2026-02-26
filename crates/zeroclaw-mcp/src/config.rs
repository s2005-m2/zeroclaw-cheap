//! MCP server configuration parsing
//!
//! Parses `.mcp.json` config files for MCP server definitions.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// MCP server configuration entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Internal structure for deserializing .mcp.json
#[derive(Debug, Deserialize, Serialize)]
struct McpConfigFile {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, McpServerEntry>,
}

/// Internal structure for individual server entries
#[derive(Debug, Deserialize, Serialize)]
struct McpServerEntry {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

/// Parse .mcp.json file at given path.
/// Returns empty vec if file doesn't exist.
pub fn parse_mcp_config(path: &Path) -> anyhow::Result<Vec<McpServerConfig>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("Failed to read {:?}", path)),
    };

    let config_file: McpConfigFile = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse {:?} as JSON", path))?;

    let mut configs = Vec::with_capacity(config_file.mcp_servers.len());
    for (name, entry) in config_file.mcp_servers {
        configs.push(McpServerConfig {
            name,
            command: entry.command,
            args: entry.args,
            env: entry.env,
        });
    }

    Ok(configs)
}

/// Load MCP config with search order:
/// workspace .mcp.json → ~/.zeroclaw/.mcp.json
/// Returns configs from first found file, or empty vec if none found.
pub fn load_mcp_configs(workspace_dir: Option<&Path>) -> anyhow::Result<Vec<McpServerConfig>> {
    // Check workspace directory first
    if let Some(workspace) = workspace_dir {
        let workspace_config = workspace.join(".mcp.json");
        if workspace_config.exists() {
            return parse_mcp_config(&workspace_config);
        }
    }

    // Check global config directory
    if let Some(home_dir) = dirs::home_dir() {
        let global_config = home_dir.join(".zeroclaw").join(".mcp.json");
        if global_config.exists() {
            return parse_mcp_config(&global_config);
        }
    }

    Ok(Vec::new())
}

/// Save MCP server configurations to a .mcp.json file.
/// Uses atomic write (write to .tmp, then rename) to avoid partial writes.
pub fn save_mcp_config(
    path: &Path,
    servers: &HashMap<String, McpServerConfig>,
) -> anyhow::Result<()> {
    // Convert HashMap<String, McpServerConfig> to the .mcp.json format:
    // {"mcpServers": {"name": {"command": "...", "args": [...], "env": {...}}}}
    let mut entries = HashMap::with_capacity(servers.len());
    for (name, config) in servers {
        entries.insert(
            name.clone(),
            McpServerEntry {
                command: config.command.clone(),
                args: config.args.clone(),
                env: config.env.clone(),
            },
        );
    }

    let file = McpConfigFile {
        mcp_servers: entries,
    };

    let json = serde_json::to_string_pretty(&file).context("Failed to serialize MCP config")?;

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, json.as_bytes())
        .with_context(|| format!("Failed to write temporary MCP config to {:?}", tmp_path))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to rename {:?} to {:?}", tmp_path, path))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_temp_config(content: &str) -> (std::path::PathBuf, TempDir) {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_path = temp_dir.path().join(".mcp.json");
        fs::write(&config_path, content).expect("Failed to write config");
        (config_path, temp_dir)
    }

    #[test]
    fn test_parse_valid_config_single_server() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
                    "env": {"NODE_ENV": "production"}
                }
            }
        }"#;
        let (config_path, _temp_dir) = create_temp_config(json);
        let configs = parse_mcp_config(&config_path).expect("Failed to parse config");

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "filesystem");
        assert_eq!(configs[0].command, "npx");
        assert_eq!(
            configs[0].args,
            vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        );
        assert_eq!(
            configs[0].env.get("NODE_ENV"),
            Some(&"production".to_string())
        );
    }

    #[test]
    fn test_parse_valid_config_multiple_servers() {
        let json = r#"{
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
                },
                "git": {
                    "command": "node",
                    "args": ["/path/to/git-server.js"],
                    "env": {"DEBUG": "true"}
                }
            }
        }"#;
        let (config_path, _temp_dir) = create_temp_config(json);
        let configs = parse_mcp_config(&config_path).expect("Failed to parse config");

        assert_eq!(configs.len(), 2);
        let fs_config = configs
            .iter()
            .find(|c| c.name == "filesystem")
            .expect("Missing filesystem server");
        let git_config = configs
            .iter()
            .find(|c| c.name == "git")
            .expect("Missing git server");

        assert_eq!(fs_config.command, "npx");
        assert_eq!(git_config.command, "node");
    }

    #[test]
    fn test_parse_empty_mcp_servers() {
        let json = r#"{"mcpServers": {}}"#;
        let (config_path, _temp_dir) = create_temp_config(json);
        let configs = parse_mcp_config(&config_path).expect("Failed to parse config");
        assert!(configs.is_empty());
    }

    #[test]
    fn test_parse_missing_file_returns_empty() {
        let configs = parse_mcp_config(Path::new("/nonexistent/.mcp.json"))
            .expect("Should not error on missing file");
        assert!(configs.is_empty());
    }

    #[test]
    fn test_parse_invalid_json_returns_error() {
        let json = r#"{"mcpServers": invalid json}"#;
        let (config_path, _temp_dir) = create_temp_config(json);
        let result = parse_mcp_config(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_server_without_args_env_defaults_to_empty() {
        let json = r#"{
            "mcpServers": {
                "simple": {
                    "command": "echo"
                }
            }
        }"#;
        let (config_path, _temp_dir) = create_temp_config(json);
        let configs = parse_mcp_config(&config_path).expect("Failed to parse config");

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].command, "echo");
        assert!(configs[0].args.is_empty());
        assert!(configs[0].env.is_empty());
    }

    #[test]
    fn test_load_mcp_configs_workspace_priority() {
        let workspace_json = r#"{"mcpServers": {"workspace": {"command": "workspace-cmd"}}}"#;
        let global_json = r#"{"mcpServers": {"global": {"command": "global-cmd"}}}"#;

        let workspace_dir = TempDir::new().expect("Failed to create workspace dir");
        let global_dir = TempDir::new().expect("Failed to create global dir");

        fs::write(workspace_dir.path().join(".mcp.json"), workspace_json)
            .expect("Failed to write workspace config");
        fs::create_dir_all(global_dir.path().join(".zeroclaw"))
            .expect("Failed to create .zeroclaw dir");
        fs::write(
            global_dir.path().join(".zeroclaw").join(".mcp.json"),
            global_json,
        )
        .expect("Failed to write global config");

        // Temporarily override home dir for testing
        let configs = load_mcp_configs(Some(workspace_dir.path())).expect("Failed to load configs");

        // Should find workspace config first
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "workspace");
    }
}
