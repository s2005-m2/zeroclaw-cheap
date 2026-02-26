//! Tool exposing Feishu Docs sync operations to the agent.
//!
//! Allows the agent to query sync status, list/add/remove synced files,
//! and trigger push/pull operations.

use super::traits::{Tool, ToolResult};
use crate::config::Config;
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use sha2::{Digest, Sha256};

pub struct DocsSyncTool {
    config: Arc<Config>,
    security: Arc<SecurityPolicy>,
    workspace_dir: PathBuf,
}
impl DocsSyncTool {
    pub fn new(config: Arc<Config>, security: Arc<SecurityPolicy>, workspace_dir: PathBuf) -> Self {
        Self {
            config,
            security,
            workspace_dir,
        }
    }

    fn require_write_access(&self) -> Option<ToolResult> {
        if !self.security.can_act() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }
        if !self.security.record_action() {
            return Some(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }
        None
    }
}

#[async_trait]
impl Tool for DocsSyncTool {
    fn name(&self) -> &str {
        "docs_sync"
    }

    fn description(&self) -> &str {
        "Manage Feishu Docs bidirectional sync. Actions: status, list, add, remove, push, pull. \
         Use this tool to check which files are synced to Feishu, add/remove files from the sync list, \
         or trigger a manual push/pull."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["status", "list", "add", "remove", "push", "pull"],
                    "description": "Action to perform. status: show sync config and state. list: show synced files. add: add a file to sync list. remove: remove a file from sync list. push: push local files to Feishu now. pull: pull remote document to local now."
                },
                "file": {
                    "type": "string",
                    "description": "File path (relative to workspace) for add/remove actions."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
        let ds = &self.config.docs_sync;
        match action {
            "status" => self.action_status(ds),
            "list" => self.action_list(ds),
            "add" => self.action_add(file).await,
            "remove" => self.action_remove(file).await,
            "push" => self.action_push(ds).await,
            "pull" => self.action_pull(ds).await,
            other => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Unknown action: {other}")),
            }),
        }
    }
}
impl DocsSyncTool {
    fn action_status(&self, ds: &crate::config::DocsSyncConfig) -> anyhow::Result<ToolResult> {
        let status = json!({
            "enabled": ds.enabled,
            "document_id": ds.document_id,
            "remote_mode": format!("{:?}", ds.remote_mode),
            "sync_interval_secs": ds.sync_interval_secs,
            "sync_files": ds.sync_files,
            "has_app_id": ds.app_id.is_some(),
            "has_app_secret": ds.app_secret.is_some(),
        });
        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&status).unwrap_or_default(),
            error: None,
        })
    }
    fn action_list(&self, ds: &crate::config::DocsSyncConfig) -> anyhow::Result<ToolResult> {
        if !ds.enabled {
            return Ok(ToolResult {
                success: true,
                output: "Docs sync is disabled. No files are being synced.".into(),
                error: None,
            });
        }
        let mut out = String::from("Synced files:\n");
        for f in &ds.sync_files {
            let path = self.workspace_dir.join(f);
            let exists = path.exists();
            out.push_str(&format!("  - {f} (exists: {exists})\n"));
        }
        Ok(ToolResult { success: true, output: out, error: None })
    }
    async fn action_add(&self, file: &str) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = self.require_write_access() {
            return Ok(blocked);
        }
        if file.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Missing 'file' parameter for add action.".into()),
            });
        }
        // Read current config from disk, modify, and save
        let contents = std::fs::read_to_string(&self.config.config_path)?;
        let mut parsed: Config = toml::from_str(&contents)?;
        parsed.config_path = self.config.config_path.clone();
        parsed.workspace_dir = self.config.workspace_dir.clone();
        if parsed.docs_sync.sync_files.iter().any(|f| f == file) {
            return Ok(ToolResult {
                success: true,
                output: format!("{file} is already in the sync list."),
                error: None,
            });
        }
        parsed.docs_sync.sync_files.push(file.to_string());
        parsed.save().await?;
        Ok(ToolResult {
            success: true,
            output: format!("Added {file} to sync list."),
            error: None,
        })
    }
    async fn action_remove(&self, file: &str) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = self.require_write_access() {
            return Ok(blocked);
        }
        if file.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Missing 'file' parameter for remove action.".into()),
            });
        }
        let contents = std::fs::read_to_string(&self.config.config_path)?;
        let mut parsed: Config = toml::from_str(&contents)?;
        parsed.config_path = self.config.config_path.clone();
        parsed.workspace_dir = self.config.workspace_dir.clone();
        let before = parsed.docs_sync.sync_files.len();
        parsed.docs_sync.sync_files.retain(|f| f != file);
        if parsed.docs_sync.sync_files.len() == before {
            return Ok(ToolResult {
                success: true,
                output: format!("{file} is not in the sync list."),
                error: None,
            });
        }
        parsed.save().await?;
        Ok(ToolResult {
            success: true,
            output: format!("Removed {file} from sync list."),
            error: None,
        })
    }
    async fn action_push(&self, ds: &crate::config::DocsSyncConfig) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = self.require_write_access() {
            return Ok(blocked);
        }
        if !ds.enabled {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Docs sync is disabled.".into()),
            });
        }
        if ds.document_ids.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("No document_ids configured.".into()),
            });
        }
        let app_id = ds.app_id.clone()
            .or_else(|| self.config.channels_config.feishu.as_ref().map(|f| f.app_id.clone()))
            .or_else(|| self.config.channels_config.lark.as_ref().map(|l| l.app_id.clone()));
        let app_secret = ds.app_secret.clone()
            .or_else(|| self.config.channels_config.feishu.as_ref().map(|f| f.app_secret.clone()))
            .or_else(|| self.config.channels_config.lark.as_ref().map(|l| l.app_secret.clone()));
        let (app_id, app_secret) = match (app_id, app_secret) {
            (Some(id), Some(secret)) => (id, secret),
            _ => return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("No app_id/app_secret found in [docs_sync] or [channels_config.feishu].".into()),
            }),
        };
        let client = crate::docs_sync::FeishuDocsClient::new(app_id, app_secret);
        let mut pushed = Vec::new();
        for (filename, doc_id) in &ds.document_ids {
            let content = match crate::docs_sync::sync::read_single_file(filename, &self.workspace_dir)? {
                Some(c) => c,
                None => continue,
            };
            let blocks = client.get_document_blocks(doc_id).await?;
            let target_block_id = blocks.iter()
                .find(|(_, bt)| *bt == 14)
                .or_else(|| blocks.iter().find(|(_, bt)| *bt == 1))
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| doc_id.clone());
            let update = crate::docs_sync::client::BlockUpdate {
                block_id: target_block_id,
                update_text_elements: serde_json::json!({
                    "elements": [{ "text_run": { "content": content } }]
                }),
            };
            client.batch_update_blocks(doc_id, &[update]).await?;
            pushed.push(filename.as_str());
        }
        Ok(ToolResult {
            success: true,
            output: format!("Pushed {} file(s): {}", pushed.len(), pushed.join(", ")),
            error: None,
        })
    }
    async fn action_pull(&self, ds: &crate::config::DocsSyncConfig) -> anyhow::Result<ToolResult> {
        if let Some(blocked) = self.require_write_access() {
            return Ok(blocked);
        }
        if !ds.enabled {
            return Ok(ToolResult {
                success: false, output: String::new(),
                error: Some("Docs sync is disabled.".into()),
            });
        }
        if ds.document_ids.is_empty() {
            return Ok(ToolResult {
                success: false, output: String::new(),
                error: Some("No document_ids configured.".into()),
            });
        }
        let app_id = ds.app_id.clone()
            .or_else(|| self.config.channels_config.feishu.as_ref().map(|f| f.app_id.clone()))
            .or_else(|| self.config.channels_config.lark.as_ref().map(|l| l.app_id.clone()));
        let app_secret = ds.app_secret.clone()
            .or_else(|| self.config.channels_config.feishu.as_ref().map(|f| f.app_secret.clone()))
            .or_else(|| self.config.channels_config.lark.as_ref().map(|l| l.app_secret.clone()));
        let (app_id, app_secret) = match (app_id, app_secret) {
            (Some(id), Some(secret)) => (id, secret),
            _ => return Ok(ToolResult {
                success: false, output: String::new(),
                error: Some("No app_id/app_secret found.".into()),
            }),
        };
        let client = crate::docs_sync::FeishuDocsClient::new(app_id, app_secret);
        // Load lock file for hash dedup and to register newly pulled files
        let lock_path = self.config.config_path.parent()
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
            .join("docs_sync.lock");
        let mut lock: std::collections::HashMap<String, serde_json::Value> =
            std::fs::read_to_string(&lock_path).ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
        let mut updated = Vec::new();
        for (filename, doc_id) in &ds.document_ids {
            let raw = client.get_raw_content(doc_id).await?;
            let target = self.workspace_dir.join(filename);
            // Reject symlinks but allow pulling new files (agent-initiated pull)
            if target.is_symlink() { continue; }
            if filename == "config.toml" {
                crate::docs_sync::sync::validate_remote_config(&raw)?;
            }
            // Hash check: skip if content unchanged
            let mut hasher = Sha256::new();
            hasher.update(raw.as_bytes());
            let hash = format!("{:x}", hasher.finalize());
            if let Some(entry) = lock.get(filename) {
                if entry.get("hash").and_then(|v| v.as_str()) == Some(&hash) {
                    continue;
                }
            }
            // Create parent dir if needed (for new files)
            if let Some(parent) = target.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::write(&target, &raw)?;
            // Update lock entry
            lock.insert(filename.clone(), json!({
                "doc_id": doc_id,
                "hash": hash,
            }));
            updated.push(filename.as_str());
        }
        // Persist lock
        if !updated.is_empty() {
            let _ = std::fs::write(&lock_path, serde_json::to_string_pretty(&lock).unwrap_or_default());
        }
        if updated.is_empty() {
            Ok(ToolResult { success: true, output: "Pull complete. No files updated.".into(), error: None })
        } else {
            Ok(ToolResult {
                success: true,
                output: format!("Pull complete. Updated: {}", updated.join(", ")),
                error: None,
            })
        }
    }
}
