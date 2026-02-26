//! Feishu Docs bidirectional sync module.
//!
//! Enables syncing local config files (config.toml, IDENTITY.md, etc.)
//! to/from a Feishu document. Gated behind `feishu-docs-sync` feature.

pub mod client;
pub mod sync;
pub mod watcher;
pub mod event_subscriber;
pub mod worker;

pub use client::{FeishuDocsClient, BlockUpdate};
pub use sync::{sync_local_to_remote, sync_remote_to_local, validate_remote_config};
pub use watcher::FileWatcher;
pub use event_subscriber::EventSubscriber;
pub use worker::run as run_worker;

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use anyhow::Result;

// ── DocsSyncSharer ──

#[derive(serde::Deserialize)]
struct SyncLockEntry {
    doc_id: String,
}

/// Shares docs_sync documents with Feishu users via permission API.
/// Used by LarkChannel to auto-share on first user message.
pub struct DocsSyncSharer {
    client: FeishuDocsClient,
    lock_path: PathBuf,
    shared_users_path: PathBuf,
}

fn load_shared_users(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_shared_users(path: &Path, users: &[String]) -> Result<()> {
    let json = serde_json::to_string_pretty(users)?;
    std::fs::write(path, json)?;
    Ok(())
}

impl DocsSyncSharer {
    /// Create a new sharer from Feishu app credentials and the lock file path.
    pub fn new(app_id: String, app_secret: String, lock_path: PathBuf) -> Self {
        let client = FeishuDocsClient::new(app_id, app_secret);
        let shared_users_path = lock_path
            .parent()
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
            .join("docs_sync_shared_users.json");
        Self {
            client,
            lock_path,
            shared_users_path,
        }
    }

    /// Share all synced documents with a Feishu user by open_id.
    /// Skips if the user was already shared with. Logs and continues on per-doc failures.
    pub async fn share_all_docs_with(&self, open_id: &str) -> Result<()> {
        // Check if user already has shared docs
        let mut users = load_shared_users(&self.shared_users_path);
        if users.iter().any(|u| u == open_id) {
            tracing::debug!("docs_sync: user {open_id} already has shared docs, skipping");
            return Ok(());
        }

        // Load lock file to get doc_ids
        let lock: HashMap<String, SyncLockEntry> = std::fs::read_to_string(&self.lock_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let doc_count = lock.len();
        tracing::info!("docs_sync: sharing {doc_count} docs with user {open_id}");

        let mut had_failure = false;
        for (_filename, entry) in &lock {
            let doc_id = &entry.doc_id;
            if let Err(e) = self.client.add_permission_member(doc_id, open_id, "view").await {
                tracing::warn!("docs_sync: failed to share doc {doc_id} with {open_id}: {e}");
                had_failure = true;
            }
        }

        // Only record user as shared if all docs succeeded — allows retry on next message
        if had_failure {
            tracing::warn!("docs_sync: some docs failed to share with {open_id}, will retry next message");
        } else {
            users.push(open_id.to_owned());
            save_shared_users(&self.shared_users_path, &users)?;
            tracing::info!("docs_sync: shared all docs with user {open_id}");
        }
        Ok(())
    }
}
