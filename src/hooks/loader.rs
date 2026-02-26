use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use tracing::warn;

use crate::config::schema::HooksConfig;
use crate::hooks::audit::audit_hook_directory;
use crate::hooks::manifest::HookManifest;

/// A successfully loaded hook: parsed manifest + directory path.
#[derive(Debug, Clone)]
pub struct LoadedHook {
    pub manifest: HookManifest,
    pub hook_dir: PathBuf,
}

/// Load all valid hooks from a hooks directory.
///
/// Each subdirectory of `hooks_dir` is expected to contain a `HOOK.toml` manifest.
/// Invalid hooks are skipped with a warning. Results are sorted by priority (descending).
/// Enforces `config.max_hooks` limit.
pub fn load_hooks_from_dir(hooks_dir: &Path, config: &HooksConfig) -> Result<Vec<LoadedHook>> {
    if !hooks_dir.exists() {
        bail!("hooks directory does not exist: {}", hooks_dir.display());
    }
    if !hooks_dir.is_dir() {
        bail!("hooks path is not a directory: {}", hooks_dir.display());
    }

    let mut hooks = Vec::new();

    let entries = std::fs::read_dir(hooks_dir).map_err(|e| {
        anyhow::anyhow!(
            "failed to read hooks directory {}: {e}",
            hooks_dir.display()
        )
    })?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    "failed to read directory entry in {}: {e}",
                    hooks_dir.display()
                );
                continue;
            }
        };

        let sub_dir = entry.path();
        if !sub_dir.is_dir() {
            continue;
        }

        let manifest_path = sub_dir.join("HOOK.toml");
        if !manifest_path.is_file() {
            warn!(
                "skipping hook directory {}: no HOOK.toml found",
                sub_dir.display()
            );
            continue;
        }

        // Parse manifest
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    "skipping hook {}: failed to read HOOK.toml: {e}",
                    sub_dir.display()
                );
                continue;
            }
        };

        let manifest = match HookManifest::from_toml(&content) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "skipping hook {}: invalid HOOK.toml: {e}",
                    sub_dir.display()
                );
                continue;
            }
        };

        // Run security audit
        let skip_audit = config.skip_security_audit || manifest.skip_security_audit;
        match audit_hook_directory(&sub_dir, skip_audit) {
            Ok(report) => {
                if !report.is_clean() {
                    warn!(
                        "skipping hook {}: security audit failed: {}",
                        manifest.name,
                        report.summary()
                    );
                    continue;
                }
            }
            Err(e) => {
                warn!("skipping hook {}: audit error: {e}", sub_dir.display());
                continue;
            }
        }

        hooks.push(LoadedHook {
            manifest,
            hook_dir: sub_dir,
        });
    }

    // Sort by priority descending
    hooks.sort_by(|a, b| b.manifest.priority.cmp(&a.manifest.priority));

    // Enforce max_hooks limit
    hooks.truncate(config.max_hooks);

    Ok(hooks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_config() -> HooksConfig {
        HooksConfig {
            enabled: true,
            builtin: Default::default(),
            hooks_dir: None,
            skip_security_audit: true, // skip audit in tests for simplicity
            max_hooks: 50,
            default_timeout_secs: 30,
        }
    }

    fn write_hook_toml(dir: &Path, name: &str, event: &str, priority: i32) {
        let hook_dir = dir.join(name);
        std::fs::create_dir_all(&hook_dir).unwrap();
        let toml = format!(
            r#"[hook]
name = "{name}"
event = "{event}"
priority = {priority}
[hook.action.shell]
command = "echo hello"
"#
        );
        std::fs::write(hook_dir.join("HOOK.toml"), toml).unwrap();
    }
    #[test]
    fn load_single_valid_hook() {
        let dir = tempdir().unwrap();
        write_hook_toml(dir.path(), "my-hook", "on_session_start", 5);
        let config = test_config();
        let hooks = load_hooks_from_dir(dir.path(), &config).unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].manifest.name, "my-hook");
        assert_eq!(hooks[0].manifest.priority, 5);
    }
    #[test]
    fn load_multiple_hooks_sorted_by_priority() {
        let dir = tempdir().unwrap();
        write_hook_toml(dir.path(), "low", "on_session_start", 1);
        write_hook_toml(dir.path(), "high", "on_session_end", 10);
        write_hook_toml(dir.path(), "mid", "on_heartbeat_tick", 5);
        let config = test_config();
        let hooks = load_hooks_from_dir(dir.path(), &config).unwrap();
        assert_eq!(hooks.len(), 3);
        assert_eq!(hooks[0].manifest.name, "high");
        assert_eq!(hooks[1].manifest.name, "mid");
        assert_eq!(hooks[2].manifest.name, "low");
    }
    #[test]
    fn skip_malformed_hook_toml() {
        let dir = tempdir().unwrap();
        write_hook_toml(dir.path(), "good", "on_session_start", 5);
        // Create a malformed hook
        let bad_dir = dir.path().join("bad-hook");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("HOOK.toml"), "not valid toml {{{").unwrap();
        let config = test_config();
        let hooks = load_hooks_from_dir(dir.path(), &config).unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].manifest.name, "good");
    }
    #[test]
    fn enforce_max_hooks_limit() {
        let dir = tempdir().unwrap();
        write_hook_toml(dir.path(), "hook-a", "on_session_start", 10);
        write_hook_toml(dir.path(), "hook-b", "on_session_end", 5);
        write_hook_toml(dir.path(), "hook-c", "on_heartbeat_tick", 1);
        let mut config = test_config();
        config.max_hooks = 2;
        let hooks = load_hooks_from_dir(dir.path(), &config).unwrap();
        assert_eq!(hooks.len(), 2);
        // Highest priority hooks kept
        assert_eq!(hooks[0].manifest.priority, 10);
        assert_eq!(hooks[1].manifest.priority, 5);
    }
    #[test]
    fn empty_directory_returns_empty_vec() {
        let dir = tempdir().unwrap();
        let config = test_config();
        let hooks = load_hooks_from_dir(dir.path(), &config).unwrap();
        assert!(hooks.is_empty());
    }
    #[test]
    fn nonexistent_directory_returns_error() {
        let config = test_config();
        let result = load_hooks_from_dir(Path::new("/nonexistent/hooks/dir"), &config);
        assert!(result.is_err());
    }
    #[test]
    fn skips_non_directory_entries() {
        let dir = tempdir().unwrap();
        write_hook_toml(dir.path(), "good", "on_session_start", 5);
        // Create a plain file (not a directory) at top level
        std::fs::write(dir.path().join("not-a-dir.txt"), "hello").unwrap();
        let config = test_config();
        let hooks = load_hooks_from_dir(dir.path(), &config).unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].manifest.name, "good");
    }
}
