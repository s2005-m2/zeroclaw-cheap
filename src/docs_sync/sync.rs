//! Bidirectional sync engine for Feishu Docs ↔ local files.
//!
//! Security: remote changes to `[security]`, `[gateway]`, and `[autonomy]`
//! sections in config.toml are rejected.

use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::Path;

/// TOML section headers that must never be overwritten from remote.
const FORBIDDEN_SECTIONS: &[&str] = &["[security]", "[gateway]", "[autonomy]"];

/// Parse code blocks from a Feishu document's raw content.
///
/// Expected format:
/// ```
/// === config.toml ===
/// <file content>
/// === end ===
/// ```
pub fn parse_code_blocks(doc_content: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut current_file: Option<String> = None;
    let mut current_content = String::new();

    for line in doc_content.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed
            .strip_prefix("=== ")
            .and_then(|s| s.strip_suffix(" ==="))
        {
            if name == "end" {
                if let Some(file_name) = current_file.take() {
                    // Trim trailing newline from accumulated content
                    let content = current_content.trim_end_matches('\n').to_string();
                    result.insert(file_name, content);
                }
                current_content.clear();
            } else {
                current_file = Some(name.to_string());
                current_content.clear();
            }
        } else if current_file.is_some() {
            if !current_content.is_empty() {
                current_content.push('\n');
            }
            current_content.push_str(line);
        }
    }

    result
}

/// Serialize local files into code-block format for Feishu document.
pub fn serialize_to_code_blocks(files: &HashMap<String, String>) -> String {
    let mut output = String::new();
    let mut sorted_keys: Vec<&String> = files.keys().collect();
    sorted_keys.sort();
    for key in sorted_keys {
        let content = &files[key];
        output.push_str(&format!("=== {key} ===\n"));
        output.push_str(content);
        if !content.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("=== end ===\n");
    }
    output
}

/// Validate that remote config.toml content does not modify forbidden sections.
///
/// Returns `Err` if the remote content contains `[security]`, `[gateway]`, or
/// `[autonomy]` section headers.
pub fn validate_remote_config(content: &str) -> Result<()> {
    for line in content.lines() {
        let trimmed = line.trim();
        for forbidden in FORBIDDEN_SECTIONS {
            if trimmed == *forbidden {
                bail!(
                    "Remote config contains forbidden section {forbidden}; rejecting sync"
                );
            }
        }
    }
    Ok(())
}
/// Pull remote document content to local files.
///
/// Only writes files listed in `sync_files`. Rejects symlinks.
/// For `config.toml`, validates that forbidden sections are not present.
/// Returns the list of files that were updated.
pub fn sync_remote_to_local(
    remote_content: &str,
    sync_files: &[String],
    workspace: &Path,
) -> Result<Vec<String>> {
    let blocks = parse_code_blocks(remote_content);
    let mut updated = Vec::new();

    for (filename, content) in &blocks {
        // Only write files in the configured sync list
        if !sync_files.iter().any(|f| f == filename) {
            tracing::warn!("docs_sync: ignoring unlisted file '{filename}' from remote");
            continue;
        }

        let target = workspace.join(filename);

        // Reject symlinks
        if target.is_symlink() {
            tracing::warn!("docs_sync: refusing to write symlink target '{filename}'");
            continue;
        }

        // Security: reject forbidden sections in config.toml
        if filename == "config.toml" {
            validate_remote_config(content)?;
        }

        std::fs::write(&target, content)?;
        updated.push(filename.clone());
    }

    Ok(updated)
}
/// Push local files to remote document format.
///
/// Reads each file in `sync_files` from `workspace` and serializes
/// them into code-block format. Skips symlinks and missing files.
/// Returns the serialized content string.
pub fn sync_local_to_remote(
    sync_files: &[String],
    workspace: &Path,
) -> Result<String> {
    let mut files = HashMap::new();
    for filename in sync_files {
        let source = workspace.join(filename);
        // Skip symlinks
        if source.is_symlink() {
            tracing::warn!("docs_sync: skipping symlink '{filename}' for upload");
            continue;
        }
        if !source.exists() {
            tracing::debug!("docs_sync: skipping missing file '{filename}'");
            continue;
        }
        let content = std::fs::read_to_string(&source)?;
        files.insert(filename.clone(), content);
    }
    Ok(serialize_to_code_blocks(&files))
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_block_roundtrip() {
        let mut files = HashMap::new();
        files.insert("config.toml".to_string(), "[agent]\ncompact_context = true".to_string());
        files.insert("IDENTITY.md".to_string(), "# Identity\nI am ZeroClaw.".to_string());

        let serialized = serialize_to_code_blocks(&files);
        let parsed = parse_code_blocks(&serialized);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed["config.toml"], "[agent]\ncompact_context = true");
        assert_eq!(parsed["IDENTITY.md"], "# Identity\nI am ZeroClaw.");
    }
    #[test]
    fn test_validate_remote_config_rejects_security() {
        let content = "[agent]\ncompact_context = true\n[security]\nsome_key = true";
        assert!(validate_remote_config(content).is_err());
    }
    #[test]
    fn test_validate_remote_config_rejects_gateway() {
        let content = "[gateway]\nport = 9999";
        assert!(validate_remote_config(content).is_err());
    }
    #[test]
    fn test_validate_remote_config_rejects_autonomy() {
        let content = "[autonomy]\nlevel = \"full\"";
        assert!(validate_remote_config(content).is_err());
    }
    #[test]
    fn test_validate_remote_config_allows_safe_content() {
        let content = "[agent]\ncompact_context = true\n[memory]\nbackend = \"sqlite\"";
        assert!(validate_remote_config(content).is_ok());
    }
    #[test]
    fn test_config_defaults() {
        let config = crate::config::schema::DocsSyncConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.sync_files.len(), 5);
        assert_eq!(config.sync_interval_secs, 60);
        assert!(config.document_id.is_empty());
        assert!(!config.auto_create_doc);
        assert_eq!(config.remote_mode, crate::config::schema::RemoteSyncMode::Polling);
        assert!(config.app_id.is_none());
        assert!(config.app_secret.is_none());
        assert!(config.encrypt_key.is_none());
    }
    #[test]
    fn test_remote_sync_mode_serde() {
        use crate::config::schema::RemoteSyncMode;
        let polling: RemoteSyncMode = serde_json::from_str("\"polling\"").unwrap();
        assert_eq!(polling, RemoteSyncMode::Polling);
        let event: RemoteSyncMode = serde_json::from_str("\"event\"").unwrap();
        assert_eq!(event, RemoteSyncMode::Event);
    }
    #[test]
    fn test_docs_sync_config_with_event_mode() {
        let json = r#"{
            "enabled": true,
            "document_id": "doxcn123",
            "remote_mode": "event",
            "app_id": "cli_test",
            "app_secret": "secret_test",
            "encrypt_key": "enc_key"
        }"#;
        let config: crate::config::schema::DocsSyncConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.document_id, "doxcn123");
        assert_eq!(config.remote_mode, crate::config::schema::RemoteSyncMode::Event);
        assert_eq!(config.app_id.as_deref(), Some("cli_test"));
        assert_eq!(config.app_secret.as_deref(), Some("secret_test"));
        assert_eq!(config.encrypt_key.as_deref(), Some("enc_key"));
    }
    #[test]
    fn test_docs_sync_config_defaults_to_polling() {
        let json = r#"{ "enabled": true, "document_id": "doxcn456" }"#;
        let config: crate::config::schema::DocsSyncConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.remote_mode, crate::config::schema::RemoteSyncMode::Polling);
        assert!(config.app_id.is_none());
        assert!(config.app_secret.is_none());
        assert!(config.encrypt_key.is_none());
    }
}
