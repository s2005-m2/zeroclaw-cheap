//! Daemon worker for Feishu Docs bidirectional sync.
//!
//! Each local file maps 1:1 to a Feishu document.
//! Direction rule: local files are source-of-truth for *existence*.
//! - Push: only uploads files that exist locally. Auto-creates Feishu doc if needed.
//! - Pull: only updates files that already exist locally. Never creates new local files.
//!
//! Lock file (`docs_sync.lock`) tracks `{filename: {doc_id, hash}}`.

use crate::config::Config;
use anyhow::{bail, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::time::Duration;

/// Lock file name, stored next to config.toml.
const LOCK_FILENAME: &str = "docs_sync.lock";

// ── Lock entry ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LockEntry {
    doc_id: String,
    hash: String,
}

/// Lock file: `{ "IDENTITY.md": { "doc_id": "doxcn_abc", "hash": "sha256hex" }, ... }`
type LockMap = HashMap<String, LockEntry>;
// ── Helpers ──

fn lock_file_path(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(LOCK_FILENAME)
}

fn load_lock(path: &Path) -> LockMap {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_lock(path: &Path, lock: &LockMap) -> Result<()> {
    let json = serde_json::to_string_pretty(lock)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
// ── Credential resolution ──
fn resolve_credentials(config: &Config) -> Result<(String, String)> {
    let ds = &config.docs_sync;
    let app_id = ds.app_id.clone()
        .or_else(|| config.channels_config.feishu.as_ref().map(|f| f.app_id.clone()))
        .or_else(|| config.channels_config.lark.as_ref().map(|l| l.app_id.clone()));
    let app_secret = ds.app_secret.clone()
        .or_else(|| config.channels_config.feishu.as_ref().map(|f| f.app_secret.clone()))
        .or_else(|| config.channels_config.lark.as_ref().map(|l| l.app_secret.clone()));
    match (app_id, app_secret) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => bail!("docs_sync: no app_id/app_secret in [docs_sync] or [channels_config.feishu/lark]"),
    }
}
/// Push one local file to its Feishu document. Returns content hash.
async fn push_single_file(
    client: &super::FeishuDocsClient,
    doc_id: &str,
    content: &str,
) -> Result<()> {
    let blocks = client.get_document_blocks(doc_id).await?;
    let target = blocks.iter()
        .find(|(_, bt)| *bt == 14)
        .or_else(|| blocks.iter().find(|(_, bt)| *bt == 1))
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| doc_id.to_string());
    let update = super::BlockUpdate {
        block_id: target,
        update_text_elements: serde_json::json!({
            "elements": [{ "text_run": { "content": content } }]
        }),
    };
    client.batch_update_blocks(doc_id, &[update]).await
}
/// Main docs_sync daemon worker.
///
/// Local files are source-of-truth for existence:
/// - Push: only files that exist locally get uploaded. Auto-creates Feishu doc.
/// - Pull: only updates files that already exist locally. Never creates new files.
pub async fn run(
    config: Config,
    lark_ws_manager: Option<std::sync::Arc<crate::channels::lark_ws_manager::LarkWsManager>>,
) -> Result<()> {
    let ds = &config.docs_sync;
    if !ds.enabled {
        bail!("docs_sync: not enabled");
    }
    let (app_id, app_secret) = resolve_credentials(&config)?;
    let client = super::FeishuDocsClient::new(app_id.clone(), app_secret.clone());
    let sharer = super::DocsSyncSharer::new(app_id.clone(), app_secret.clone(), lock_file_path(&config));
    let workspace = config.workspace_dir.clone();
    let sync_files = ds.sync_files.clone();
    let sync_interval = Duration::from_secs(ds.sync_interval_secs.max(10));
    let lock_path = lock_file_path(&config);
    let mut lock: LockMap = load_lock(&lock_path);
    // ── Step 1: Initial push — only for files that exist locally ──
    // For each sync_file: if local file exists, ensure it has a Feishu doc, push if hash differs.
    // Config document_ids provides explicit mapping; missing ones get auto-created.
    tracing::info!("docs_sync: initial sync for {} configured file(s)", sync_files.len());
    for filename in &sync_files {
        // Only push files that exist locally
        let content = match super::sync::read_single_file(filename, &workspace)? {
            Some(c) => c,
            None => {
                tracing::debug!("docs_sync: '{filename}' not found locally, skipping");
                continue;
            }
        };
        let hash = sha256_hex(&content);
        // Resolve doc_id: lock > config.document_ids > auto-create
        let is_new_doc = !lock.contains_key(filename);
        let doc_id = if let Some(entry) = lock.get(filename) {
            entry.doc_id.clone()
        } else if let Some(id) = ds.document_ids.get(filename) {
            id.clone()
        } else {
            // Auto-create Feishu doc for this local file
            let title = format!("ZeroClaw - {filename}");
            let id = client.create_document(&title).await?;
            tracing::info!("docs_sync: created doc {id} for '{filename}'");
            id
        };
        // Check hash — skip if unchanged
        if let Some(entry) = lock.get(filename) {
            if entry.hash == hash && entry.doc_id == doc_id {
                tracing::debug!("docs_sync: '{filename}' unchanged, skipping");
                continue;
            }
        }
        // Push
        match push_single_file(&client, &doc_id, &content).await {
            Ok(()) => {
                tracing::info!("docs_sync: pushed '{filename}' to doc {doc_id}");
                lock.insert(filename.clone(), LockEntry { doc_id: doc_id.clone(), hash });
                let _ = save_lock(&lock_path, &lock);
                if is_new_doc {
                    sharer.share_single_doc_with_all(filename, &doc_id).await;
                }
            }
            Err(e) => tracing::warn!("docs_sync: push '{filename}' failed: {e}"),
        }
    }
    // ── Step 1b: Subscribe to edit events for all synced documents ──
    // Without per-document subscription, drive.file.edit_v1 events will NOT fire
    // even if the event is enabled in the Feishu developer console.
    for entry in lock.values() {
        if let Err(e) = client.subscribe_file_events(&entry.doc_id, "docx").await {
            tracing::warn!("docs_sync: subscribe failed for {}: {e}", entry.doc_id);
        }
    }
    // ── Step 2: Start local file watcher ──
    let watch_paths: Vec<PathBuf> = sync_files.iter().map(|f| workspace.join(f)).collect();
    let mut file_watcher = super::FileWatcher::watch(&watch_paths)?;
    tracing::info!("docs_sync: watching {} local files", sync_files.len());

    // ── Step 3: Remote change listener ──
    let (remote_tx, mut remote_rx) = tokio::sync::mpsc::channel::<()>(16);
    match ds.remote_mode {
        crate::config::schema::RemoteSyncMode::Event => {
            // WebSocket event subscriber for real-time remote edits.
            // We subscribe to ALL doc_ids in the lock — any edit triggers a pull cycle.
            let sub_doc_id = lock.values().next().map(|e| e.doc_id.clone()).unwrap_or_default();
            let manager = lark_ws_manager.clone()
                .ok_or_else(|| anyhow::anyhow!("docs_sync: LarkWsManager not provided for event mode"))?;
            let subscriber = super::EventSubscriber::new(
                manager, sub_doc_id,
            );
            let tx = remote_tx.clone();
            tokio::spawn(async move { subscriber.run(tx).await });
            tracing::info!("docs_sync: event subscriber started");
        }
        crate::config::schema::RemoteSyncMode::Polling => {
            // Polling mode: periodic ticker sends pull signal.
            let tx = remote_tx.clone();
            let interval = sync_interval;
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(interval);
                ticker.tick().await; // skip immediate first tick
                loop {
                    ticker.tick().await;
                    let _ = tx.try_send(());
                }
            });
            tracing::info!("docs_sync: polling every {}s", sync_interval.as_secs());
        }
    }
    drop(remote_tx); // only spawned tasks hold senders

    // ── Step 4: Main sync loop ──
    // Local change → hash check → push (only if file exists locally and has lock entry or is in sync_files)
    // Remote change → pull only into files that exist locally (check lock for doc_id mapping)
    tracing::info!("docs_sync: entering main sync loop");
    loop {
        tokio::select! {
            // ── Local file changed → push ──
            Some(changed_path) = file_watcher.rx.recv() => {
                // Resolve filename relative to workspace
                let filename = match changed_path.strip_prefix(&workspace) {
                    Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                    Err(_) => {
                        // Try matching by file_name against sync_files
                        match changed_path.file_name().and_then(|n| n.to_str()) {
                            Some(name) if sync_files.iter().any(|f| f == name) => name.to_string(),
                            _ => continue,
                        }
                    }
                };
                // Only push files that are in sync_files AND exist locally
                if !sync_files.iter().any(|f| f == &filename) {
                    continue;
                }
                let content = match super::sync::read_single_file(&filename, &workspace) {
                    Ok(Some(c)) => c,
                    _ => continue,
                };
                let hash = sha256_hex(&content);
                let is_new_doc = !lock.contains_key(&filename);
                let doc_id = if let Some(entry) = lock.get(&filename) {
                    // Hash unchanged? skip.
                    if entry.hash == hash {
                        tracing::debug!("docs_sync: '{filename}' unchanged, skipping push");
                        continue;
                    }
                    entry.doc_id.clone()
                } else if let Some(id) = config.docs_sync.document_ids.get(&filename) {
                    id.clone()
                } else {
                    // Auto-create Feishu doc for new local file
                    match client.create_document(&format!("ZeroClaw - {filename}")).await {
                        Ok(id) => {
                            tracing::info!("docs_sync: created doc {id} for '{filename}'");
                            id
                        }
                        Err(e) => {
                            tracing::warn!("docs_sync: create doc for '{filename}' failed: {e}");
                            continue;
                        }
                    }
                };
                match push_single_file(&client, &doc_id, &content).await {
                    Ok(()) => {
                        tracing::info!("docs_sync: pushed '{filename}'");
                        lock.insert(filename.clone(), LockEntry { doc_id: doc_id.clone(), hash });
                        let _ = save_lock(&lock_path, &lock);
                        if is_new_doc {
                            sharer.share_single_doc_with_all(&filename, &doc_id).await;
                            // Subscribe to edit events so drive.file.edit_v1 fires
                            if let Err(e) = client.subscribe_file_events(&doc_id, "docx").await {
                                tracing::warn!("docs_sync: subscribe failed for {doc_id}: {e}");
                            }
                        }
                    }
                    Err(e) => tracing::warn!("docs_sync: push failed: {e}"),
                }
            }
            // ── Remote change → pull (only existing local files) ──
            Some(()) = remote_rx.recv() => {
                tracing::debug!("docs_sync: remote change detected, pulling");
                // Iterate lock entries — only files we previously pushed have mappings.
                // Clone keys to avoid borrow conflict with lock mutation.
                let filenames: Vec<String> = lock.keys().cloned().collect();
                for filename in &filenames {
                    let entry = match lock.get(filename) {
                        Some(e) => e.clone(),
                        None => continue,
                    };
                    // SAFETY: only pull into files that exist locally
                    let local_path = workspace.join(filename);
                    if !local_path.exists() || local_path.is_symlink() {
                        tracing::debug!("docs_sync: '{filename}' not local, skipping pull");
                        continue;
                    }
                    // Fetch remote content
                    let raw = match client.get_raw_content(&entry.doc_id).await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!("docs_sync: pull '{filename}' failed: {e}");
                            continue;
                        }
                    };
                    let remote_hash = sha256_hex(&raw);
                    // Skip if remote content matches our last known hash
                    if remote_hash == entry.hash {
                        continue;
                    }
                    // Security: validate config.toml before writing
                    if filename == "config.toml" {
                        if let Err(e) = super::sync::validate_remote_config(&raw) {
                            tracing::warn!("docs_sync: pull '{filename}' blocked: {e}");
                            continue;
                        }
                    }
                    if let Err(e) = std::fs::write(&local_path, &raw) {
                        tracing::warn!("docs_sync: write '{filename}' failed: {e}");
                        continue;
                    }
                    tracing::info!("docs_sync: pulled '{filename}'");
                    lock.insert(filename.clone(), LockEntry {
                        doc_id: entry.doc_id.clone(),
                        hash: remote_hash,
                    });
                    let _ = save_lock(&lock_path, &lock);
                }
            }
            else => break,
        }
    }
    Ok(())
}
