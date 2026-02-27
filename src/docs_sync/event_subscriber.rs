//! Feishu WebSocket event subscriber for `drive.file.edit_v1`.
//!
//! Subscribes to the shared `LarkWsManager` broadcast and listens for document
//! edit events. When the synced document is edited, emits a notification on
//! a tokio channel so the sync engine can pull the latest content.

use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

use crate::channels::lark_ws_manager::LarkWsManager;

/// Feishu drive event envelope.
#[derive(Debug, serde::Deserialize)]
struct DriveEvent {
    header: DriveEventHeader,
    #[allow(dead_code)]
    event: serde_json::Value,
}

#[derive(Debug, serde::Deserialize)]
struct DriveEventHeader {
    event_type: String,
    #[allow(dead_code)]
    event_id: String,
}

/// Feishu WebSocket event subscriber.
///
/// Subscribes to the shared `LarkWsManager` broadcast, listens for
/// `drive.file.edit_v1` events, and sends a notification on `tx`
/// whenever the watched document is edited.
pub struct EventSubscriber {
    ws_manager: Arc<LarkWsManager>,
    document_id: String,
}

impl EventSubscriber {
    /// Create a new event subscriber.
    pub fn new(ws_manager: Arc<LarkWsManager>, document_id: String) -> Self {
        Self { ws_manager, document_id }
    }

    /// Run the event subscriber loop.
    ///
    /// Subscribes to the `LarkWsManager` broadcast and sends `()` on `tx`
    /// whenever a `drive.file.edit_v1` event fires for the configured document.
    /// The caller should trigger a remote-to-local sync on each receive.
    pub async fn run(&self, tx: mpsc::Sender<()>) {
        let mut rx = self.ws_manager.subscribe();
        loop {
            let event = match rx.recv().await {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("docs_sync: broadcast lagged, skipped {n} events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::error!("docs_sync: WS manager broadcast closed");
                    break;
                }
            };

            if event.event_type != "drive.file.edit_v1" {
                continue;
            }

            // Parse the event to check document_id filter
            let drive_event: DriveEvent = match serde_json::from_slice(&event.payload) {
                Ok(e) => e,
                        Err(e) => { tracing::error!("docs_sync: event JSON: {e}"); continue; }
            };

            // Filter by document_id if configured
            if !self.document_id.is_empty() {
                let file_token = drive_event.event
                    .get("file_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !file_token.is_empty() && file_token != self.document_id {
                    tracing::debug!(
                        "docs_sync: edit event for different doc {file_token}, skipping"
                    );
                    continue;
                }
            }

            tracing::info!("docs_sync: received drive.file.edit_v1 event");
            let _ = tx.try_send(());
        }
    }
}
