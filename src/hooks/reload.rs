use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tracing::warn;

const HOOKS_RELOAD_STAMP: &str = ".hooks-reload-stamp";

/// Write a reload stamp file containing the current Unix timestamp (seconds).
/// Creates parent directories if needed. Overwrites any existing stamp.
pub fn write_reload_stamp(workspace_dir: &Path) -> Result<()> {
    let stamp_path = workspace_dir.join(HOOKS_RELOAD_STAMP);
    if let Some(parent) = stamp_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs();
    std::fs::write(&stamp_path, now.to_string())?;
    Ok(())
}

/// Check whether the reload stamp has changed since the last observed value.
///
/// Returns `true` (and updates `last_stamp`) when the stamp file contains a
/// timestamp strictly greater than the cached value, or when `last_stamp` is
/// `None` and a valid stamp file exists. Returns `false` on missing file,
/// parse errors, or unchanged timestamp.
pub fn check_reload_stamp(workspace_dir: &Path, last_stamp: &mut Option<u64>) -> bool {
    let stamp_path = workspace_dir.join(HOOKS_RELOAD_STAMP);
    let content = match std::fs::read_to_string(&stamp_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let ts: u64 = match content.trim().parse() {
        Ok(v) => v,
        Err(_) => return false,
    };
    match *last_stamp {
        Some(prev) if ts <= prev => false,
        _ => {
            *last_stamp = Some(ts);
            true
        }
    }
}

/// Delete the reload stamp file. No error if the file does not exist.
pub fn delete_reload_stamp(workspace_dir: &Path) -> Result<()> {
    let stamp_path = workspace_dir.join(HOOKS_RELOAD_STAMP);
    match std::fs::remove_file(&stamp_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Perform the actual dynamic hooks reload: re-scan the hooks directory,
/// build handlers, and atomically swap them on the runner.
///
/// Called after `check_reload_stamp` returns `true`. Wraps the reload in a
/// 5-second timeout. Stamp is deleted only on success; preserved on failure
/// to allow retry next cycle.
pub async fn do_reload_hooks(
    workspace_dir: &Path,
    hooks_config: &crate::config::schema::HooksConfig,
    hook_runner: &Arc<crate::hooks::HookRunner>,
) {
    let hooks_dir = hooks_config
        .hooks_dir
        .clone()
        .unwrap_or_else(|| workspace_dir.join("hooks"));

    // Wrap the entire reload in a 5-second timeout so a slow disk scan
    // cannot stall message processing.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reload_dynamic_hooks_inner(&hooks_dir, hooks_config, hook_runner),
    )
    .await;

    match result {
        Ok(Ok(())) => {
            tracing::info!("Dynamic hooks reloaded successfully");
            // Delete stamp on success; preserve on failure to allow retry next cycle.
            if let Err(err) = delete_reload_stamp(workspace_dir) {
                warn!("Failed to delete hooks reload stamp: {err}");
            }
        }
        Ok(Err(err)) => {
            warn!("Failed to reload dynamic hooks: {err}");
        }
        Err(_elapsed) => {
            warn!("Dynamic hooks reload timed out (5s limit)");
        }
    }
}

async fn reload_dynamic_hooks_inner(
    hooks_dir: &Path,
    hooks_config: &crate::config::schema::HooksConfig,
    hook_runner: &Arc<crate::hooks::HookRunner>,
) -> Result<()> {
    let loaded = crate::hooks::loader::load_hooks_from_dir(hooks_dir, hooks_config)?;
    let handlers: Vec<Box<dyn crate::hooks::traits::HookHandler>> = loaded
        .into_iter()
        .map(|hook| {
            let timeout = hooks_config.default_timeout_secs;
            Box::new(crate::hooks::dynamic::DynamicHookHandler::new(
                hook, timeout,
            )) as Box<dyn crate::hooks::traits::HookHandler>
        })
        .collect();
    hook_runner.reload_dynamic_hooks(handlers).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_then_check_returns_true_and_updates_stamp() {
        let dir = TempDir::new().unwrap();
        write_reload_stamp(dir.path()).unwrap();

        let mut last = None;
        assert!(check_reload_stamp(dir.path(), &mut last));
        assert!(last.is_some());
    }

    #[test]
    fn check_without_stamp_file_returns_false() {
        let dir = TempDir::new().unwrap();
        let mut last = None;
        assert!(!check_reload_stamp(dir.path(), &mut last));
        assert!(last.is_none());
    }

    #[test]
    fn check_same_stamp_twice_second_returns_false() {
        let dir = TempDir::new().unwrap();
        write_reload_stamp(dir.path()).unwrap();

        let mut last = None;
        assert!(check_reload_stamp(dir.path(), &mut last));
        assert!(!check_reload_stamp(dir.path(), &mut last));
    }

    #[test]
    fn delete_stamp_works_and_no_error_if_missing() {
        let dir = TempDir::new().unwrap();
        // Delete when no file exists — should succeed
        delete_reload_stamp(dir.path()).unwrap();

        // Write then delete
        write_reload_stamp(dir.path()).unwrap();
        delete_reload_stamp(dir.path()).unwrap();

        // Confirm file is gone
        let mut last = None;
        assert!(!check_reload_stamp(dir.path(), &mut last));
    }

    #[test]
    fn newer_stamp_after_delete_and_rewrite_detected() {
        let dir = TempDir::new().unwrap();
        write_reload_stamp(dir.path()).unwrap();

        let mut last = None;
        assert!(check_reload_stamp(dir.path(), &mut last));

        // Delete and rewrite — new stamp should be >= old, but write uses
        // current time so it will be equal or greater. Force a newer value
        // by writing manually.
        let stamp_path = dir.path().join(HOOKS_RELOAD_STAMP);
        let next = last.unwrap() + 1;
        std::fs::write(&stamp_path, next.to_string()).unwrap();

        assert!(check_reload_stamp(dir.path(), &mut last));
        assert_eq!(last, Some(next));
    }
}
