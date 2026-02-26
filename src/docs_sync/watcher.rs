//! Local file watcher for Feishu Docs sync.
//!
//! Uses the `notify` crate to watch configured sync files and emit
//! changed paths via a channel. Debounces events by 500ms.

use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Duration;

/// Debounce interval for file change events.
const DEBOUNCE_DURATION: Duration = Duration::from_millis(500);

/// Local file watcher wrapping `notify::RecommendedWatcher`.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    pub rx: Receiver<PathBuf>,
}

impl FileWatcher {
    /// Watch the given file paths for changes.
    ///
    /// Returns a `FileWatcher` whose `rx` field receives debounced
    /// `PathBuf` values for each changed file. The caller is responsible
    /// for triggering sync on receive.
    pub fn watch(paths: &[PathBuf]) -> Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();

        // Track last event time per path for debouncing
        let debounce_tx = tx.clone();
        let debounce_state =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
                PathBuf,
                std::time::Instant,
            >::new()));

        let state = debounce_state.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: std::result::Result<notify::Event, _>| {
                let event = match res {
                    Ok(e) => e,
                    Err(err) => {
                        tracing::warn!("docs_sync watcher error: {err}");
                        return;
                    }
                };
                // Only care about modify/create events
                if !matches!(
                    event.kind,
                    notify::EventKind::Modify(_) | notify::EventKind::Create(_)
                ) {
                    return;
                }
                let now = std::time::Instant::now();
                let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
                for path in event.paths {
                    if let Some(last) = guard.get(&path) {
                        if now.duration_since(*last) < DEBOUNCE_DURATION {
                            continue;
                        }
                    }
                    guard.insert(path.clone(), now);
                    let _ = debounce_tx.send(path);
                }
            })?;

        for path in paths {
            if path.exists() {
                watcher.watch(path, RecursiveMode::NonRecursive)?;
            }
        }

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }
}
