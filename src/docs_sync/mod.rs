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
