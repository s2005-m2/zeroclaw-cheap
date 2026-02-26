use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::hooks::audit::audit_hook_directory;
use crate::hooks::loader::load_hooks_from_dir;
use crate::hooks::reload::write_reload_stamp;
use crate::HooksCommands;

/// Resolve the hooks directory from config or default to `{workspace}/hooks/`.
fn resolve_hooks_dir(config: &Config) -> PathBuf {
    config
        .hooks
        .hooks_dir
        .clone()
        .unwrap_or_else(|| config.workspace_dir.join("hooks"))
}

/// Handle all `zeroclaw hooks <subcommand>` CLI commands.
pub fn handle_hooks_command(command: HooksCommands, config: &Config) -> Result<()> {
    match command {
        HooksCommands::List => handle_list(config),
        HooksCommands::Reload => handle_reload(config),
        HooksCommands::Create { name } => handle_create(&name, config),
        HooksCommands::Audit { path } => handle_audit(&path, config),
    }
}

/// `zeroclaw hooks list` — print table of installed hooks (builtin + dynamic).
fn handle_list(config: &Config) -> Result<()> {
    // Builtin hooks
    println!("Builtin hooks:");
    println!(
        "  {:<20} {:<10}",
        "command-logger",
        if config.hooks.builtin.command_logger {
            "enabled"
        } else {
            "disabled"
        },
    );
    println!();

    // Dynamic hooks
    let hooks_dir = resolve_hooks_dir(config);
    if !hooks_dir.exists() {
        println!("Dynamic hooks: (none — hooks directory does not exist)");
        println!("  Directory: {}", hooks_dir.display());
        return Ok(());
    }

    match load_hooks_from_dir(&hooks_dir, &config.hooks) {
        Ok(hooks) if hooks.is_empty() => {
            println!("Dynamic hooks: (none found)");
            println!("  Directory: {}", hooks_dir.display());
        }
        Ok(hooks) => {
            println!(
                "Dynamic hooks ({}) from {}:",
                hooks.len(),
                hooks_dir.display()
            );
            println!();
            println!(
                "  {:<20} {:<25} {:<10} {:<8}",
                "NAME", "EVENT", "PRIORITY", "ENABLED"
            );
            println!("  {}", "-".repeat(65));
            for h in &hooks {
                println!(
                    "  {:<20} {:<25} {:<10} {:<8}",
                    h.manifest.name,
                    h.manifest.event.to_string(),
                    h.manifest.priority,
                    if h.manifest.enabled { "yes" } else { "no" },
                );
            }
        }
        Err(e) => {
            println!("Failed to load dynamic hooks: {e}");
            println!("  Directory: {}", hooks_dir.display());
        }
    }

    Ok(())
}
/// `zeroclaw hooks reload` — write stamp file so daemon picks up changes.
fn handle_reload(config: &Config) -> Result<()> {
    write_reload_stamp(&config.workspace_dir)?;
    println!("Reload stamp written. Daemon will reload hooks on next message.");
    Ok(())
}
/// `zeroclaw hooks create <name>` — scaffold a new hook directory with HOOK.toml.
fn handle_create(name: &str, config: &Config) -> Result<()> {
    // Validate hook name: alphanumeric + hyphens only
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        bail!("Invalid hook name '{name}': use alphanumeric characters and hyphens only.");
    }

    let hooks_dir = resolve_hooks_dir(config);
    let hook_dir = hooks_dir.join(name);
    if hook_dir.exists() {
        bail!("Hook directory already exists: {}", hook_dir.display());
    }

    std::fs::create_dir_all(&hook_dir)?;

    let template = format!(
        r#"[hook]
name = "{name}"
description = "TODO: describe what this hook does"
event = "on_session_start"
priority = 0
enabled = true

[hook.action.shell]
command = "echo hook {name} fired"
"#
    );

    let manifest_path = hook_dir.join("HOOK.toml");
    std::fs::write(&manifest_path, template)?;
    println!("Created hook scaffold: {}", manifest_path.display());
    Ok(())
}
/// `zeroclaw hooks audit <path>` — run security audit on a hook directory.
fn handle_audit(path: &str, config: &Config) -> Result<()> {
    // Resolve: if path looks like a bare name, check inside hooks_dir first
    let target = {
        let as_path = Path::new(path);
        if as_path.is_dir() {
            as_path.to_path_buf()
        } else {
            let hooks_dir = resolve_hooks_dir(config);
            let candidate = hooks_dir.join(path);
            if candidate.is_dir() {
                candidate
            } else {
                bail!(
                    "Hook directory not found: '{path}' (also checked {})",
                    hooks_dir.join(path).display()
                );
            }
        }
    };
    let report = audit_hook_directory(&target, config.hooks.skip_security_audit)?;
    if report.is_clean() {
        println!(
            "  {} Audit passed for {} ({} files scanned).",
            console::style("\u{2713}").green().bold(),
            target.display(),
            report.files_scanned,
        );
    } else {
        println!(
            "  {} Audit failed for {}",
            console::style("\u{2717}").red().bold(),
            target.display(),
        );
        for finding in &report.findings {
            println!("    - {finding}");
        }
    }
    Ok(())
}
