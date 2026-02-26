//! Daemon worker for Feishu Docs bidirectional sync.
//!
//! Spawned by the daemon supervisor when `[docs_sync].enabled = true`.
//! On startup: resolves credentials, auto-creates doc if needed, pushes local files.
//! Then runs a continuous loop: watches local file changes + polls/subscribes for remote edits.

use crate::config::Config;
use anyhow::{bail, Result};
use std::path::PathBuf;
use tokio::time::Duration;

/// Resolve app_id and app_secret from docs_sync config, falling back to feishu/lark channel config.
fn resolve_credentials(config: &Config) -> Result<(String, String)> {
    let ds = &config.docs_sync;
    let app_id = ds
        .app_id
        .clone()
        .or_else(|| {
            config
                .channels_config
                .feishu
                .as_ref()
                .map(|f| f.app_id.clone())
        })
        .or_else(|| {
            config
                .channels_config
                .lark
                .as_ref()
                .map(|l| l.app_id.clone())
        });
    let app_secret = ds
        .app_secret
        .clone()
        .or_else(|| {
            config
                .channels_config
                .feishu
                .as_ref()
                .map(|f| f.app_secret.clone())
        })
        .or_else(|| {
            config
                .channels_config
                .lark
                .as_ref()
                .map(|l| l.app_secret.clone())
        });
    match (app_id, app_secret) {
        (Some(id), Some(secret)) => Ok((id, secret)),
        _ => bail!("docs_sync: no app_id/app_secret in [docs_sync] or [channels_config.feishu/lark]"),
    }
}

/// Push local sync files to the Feishu document.
async fn push_local_to_remote(
    client: &super::FeishuDocsClient,
    document_id: &str,
    sync_files: &[String],
    workspace: &std::path::Path,
) -> Result<()> {
    let content = super::sync_local_to_remote(sync_files, workspace)?;
    if content.is_empty() {
        tracing::debug!("docs_sync: no local files to push");
        return Ok(());
    }
    let blocks = client.get_document_blocks(document_id).await?;
    // block_type 14 = Code, block_type 1 = Page
    let target_block_id = blocks
        .iter()
        .find(|(_, bt)| *bt == 14)
        .or_else(|| blocks.iter().find(|(_, bt)| *bt == 1))
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| document_id.to_string());
    let update = super::BlockUpdate {
        block_id: target_block_id,
        update_text_elements: serde_json::json!({
            "elements": [{
                "text_run": { "content": content }
            }]
        }),
    };
    client.batch_update_blocks(document_id, &[update]).await?;
    tracing::info!(
        "docs_sync: pushed {} bytes from {} file(s)",
        content.len(),
        sync_files.len()
    );
    Ok(())
}

/// Pull remote document content to local files.
async fn pull_remote_to_local(
    client: &super::FeishuDocsClient,
    document_id: &str,
    sync_files: &[String],
    workspace: &std::path::Path,
) -> Result<Vec<String>> {
    let remote_content = client.get_raw_content(document_id).await?;
    let updated = super::sync_remote_to_local(&remote_content, sync_files, workspace)?;
    if !updated.is_empty() {
        tracing::info!("docs_sync: pulled remote changes, updated: {}", updated.join(", "));
    }
    Ok(updated)
}

/// Main docs_sync daemon worker.
///
/// 1. Resolve credentials (from [docs_sync] or [channels_config.feishu/lark])
/// 2. Auto-create document if `auto_create_doc = true` and `document_id` is empty
/// 3. Initial push: local files → Feishu document
/// 4. Continuous loop: watch local files + poll/subscribe remote changes
pub async fn run(config: Config) -> Result<()> {
    let ds = &config.docs_sync;
    if !ds.enabled {
        bail!("docs_sync: not enabled");
    }

    let (app_id, app_secret) = resolve_credentials(&config)?;
    let client = super::FeishuDocsClient::new(app_id.clone(), app_secret.clone());
    let workspace = config.workspace_dir.clone();
    let sync_files = ds.sync_files.clone();
    let sync_interval = Duration::from_secs(ds.sync_interval_secs.max(10));
    // ── Step 1: Resolve or auto-create document ──
    let mut document_id = ds.document_id.clone();
    if document_id.is_empty() {
        if ds.auto_create_doc {
            tracing::info!("docs_sync: auto-creating Feishu document");
            document_id = client.create_document("ZeroClaw Sync").await?;
            tracing::info!("docs_sync: created document {document_id}");
        } else {
            bail!("docs_sync: no document_id configured and auto_create_doc is false");
        }
    }

    // ── Step 2: Initial push (local → remote) ──
    tracing::info!("docs_sync: initial push to document {document_id}");
    if let Err(e) = push_local_to_remote(&client, &document_id, &sync_files, &workspace).await {
        tracing::warn!("docs_sync: initial push failed (non-fatal): {e}");
    }

    // ── Step 3: Start local file watcher ──
    let watch_paths: Vec<PathBuf> = sync_files
        .iter()
        .map(|f| workspace.join(f))
        .collect();
    let mut file_watcher = super::FileWatcher::watch(&watch_paths)?;
    tracing::info!("docs_sync: watching {} local files for changes", sync_files.len());

    // ── Step 4: Start remote change listener ──
    let (remote_tx, mut remote_rx) = tokio::sync::mpsc::channel::<()>(16);
    let use_event_mode = matches!(ds.remote_mode, crate::config::schema::RemoteSyncMode::Event);
    if use_event_mode {
        let sub = super::EventSubscriber::new(
            app_id.clone(),
            app_secret.clone(),
            document_id.clone(),
        );
        let tx = remote_tx.clone();
        tokio::spawn(async move { sub.run(tx).await });
        tracing::info!("docs_sync: event subscriber started (WebSocket)");
    } else {
        let interval = sync_interval;
        let tx = remote_tx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                let _ = tx.try_send(());
            }
        });
        tracing::info!("docs_sync: polling mode, interval={}s", sync_interval.as_secs());
    }

    // ── Step 5: Main bidirectional sync loop ──
    tracing::info!("docs_sync: worker ready, entering sync loop");
    loop {
        tokio::select! {
            biased;
            // Local file changed → push to remote
            Some(changed_path) = file_watcher.rx.recv() => {
                tracing::info!(
                    "docs_sync: local file changed: {}",
                    changed_path.display()
                );
                if let Err(e) = push_local_to_remote(
                    &client, &document_id, &sync_files, &workspace,
                ).await {
                    tracing::warn!("docs_sync: push failed: {e}");
                }
            }
            // Remote document changed → pull to local
            Some(()) = remote_rx.recv() => {
                tracing::debug!("docs_sync: remote change notification");
                if let Err(e) = pull_remote_to_local(
                    &client, &document_id, &sync_files, &workspace,
                ).await {
                    tracing::warn!("docs_sync: pull failed: {e}");
                }
            }
        }
    }
}
