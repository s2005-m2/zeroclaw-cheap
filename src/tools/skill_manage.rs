use super::traits::{Tool, ToolResult};
use crate::skills::SkillsState;
use async_trait::async_trait;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Windows reserved device names (case-insensitive).
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// CRUD tool for managing agent skills at runtime.
pub struct SkillManageTool {
    skills_dir: PathBuf,
    shared_state: Arc<RwLock<SkillsState>>,
    workspace_dir: PathBuf,
    config: Arc<crate::config::Config>,
}

impl SkillManageTool {
    pub fn new(
        skills_dir: PathBuf,
        shared_state: Arc<RwLock<SkillsState>>,
        workspace_dir: PathBuf,
        config: Arc<crate::config::Config>,
    ) -> Self {
        Self {
            skills_dir,
            shared_state,
            workspace_dir,
            config,
        }
    }
}

/// Validate a skill name: alphanumeric start, alphanumeric/underscore/hyphen body,
/// 1-64 chars, no path traversal, no Windows reserved names.
fn validate_skill_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("Skill name must be 1-64 characters".into());
    }

    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return Err("Skill name must start with an alphanumeric character".into()),
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!(
                "Skill name contains invalid character '{c}': only alphanumeric, underscore, and hyphen allowed"
            ));
        }
    }

    // Reject path traversal components
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Skill name must not contain path traversal sequences".into());
    }

    // Reject Windows reserved device names (case-insensitive)
    let upper = name.to_uppercase();
    for reserved in WINDOWS_RESERVED {
        if upper == *reserved {
            return Err(format!(
                "Skill name '{name}' is a Windows reserved device name"
            ));
        }
    }

    Ok(())
}

#[async_trait]
impl Tool for SkillManageTool {
    fn name(&self) -> &str {
        "skill_manage"
    }

    fn description(&self) -> &str {
        "Create, read, update, delete, and list agent skills at runtime"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "read", "update", "delete", "list"],
                    "description": "The CRUD action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Skill name (required for create/read/update/delete)"
                },
                "description": {
                    "type": "string",
                    "description": "Skill description"
                },
                "version": {
                    "type": "string",
                    "description": "Skill version (default: 0.1.0)"
                },
                "tools": {
                    "type": "array",
                    "description": "Tool definitions for the skill",
                    "items": { "type": "object" }
                },
                "prompts": {
                    "type": "array",
                    "description": "Prompt strings for the skill",
                    "items": { "type": "string" }
                },
                "content": {
                    "type": "string",
                    "description": "Raw SKILL.md content (alternative to structured fields)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        match action {
            "create" => self.action_create(&args).await,
            "read" => self.action_read(&args).await,
            "update" => self.action_update(&args).await,
            "delete" => self.action_delete(&args).await,
            "list" => self.action_list().await,
            other => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Unknown action '{other}': expected create/read/update/delete/list"
                )),
            }),
        }
    }
}

impl SkillManageTool {
    /// Extract and validate the "name" field from args.
    fn extract_name(args: &serde_json::Value) -> Result<String, ToolResult> {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
            return Err(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Missing 'name' parameter for this action".into()),
            });
        }
        if let Err(e) = validate_skill_name(name) {
            return Err(ToolResult {
                success: false,
                output: String::new(),
                error: Some(e),
            });
        }
        Ok(name.to_string())
    }

    /// Build a SKILL.toml string from the provided args.
    fn build_toml(name: &str, args: &serde_json::Value) -> String {
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("A custom skill");
        let version = args
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("0.1.0");

        let mut toml = format!(
            "[skill]\nname = {name_q}\ndescription = {desc_q}\nversion = {ver_q}\n",
            name_q = toml_quote(name),
            desc_q = toml_quote(description),
            ver_q = toml_quote(version),
        );

        if let Some(tools) = args.get("tools").and_then(|v| v.as_array()) {
            for tool in tools {
                let t_name = tool
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed");
                let t_desc = tool
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let t_kind = tool.get("kind").and_then(|v| v.as_str()).unwrap_or("shell");
                let t_cmd = tool.get("command").and_then(|v| v.as_str()).unwrap_or("");
                toml.push_str(&format!(
                    "\n[[tools]]\nname = {n}\ndescription = {d}\nkind = {k}\ncommand = {c}\n",
                    n = toml_quote(t_name),
                    d = toml_quote(t_desc),
                    k = toml_quote(t_kind),
                    c = toml_quote(t_cmd),
                ));
            }
        }

        if let Some(prompts) = args.get("prompts").and_then(|v| v.as_array()) {
            toml.push_str("\nprompts = [");
            for (i, p) in prompts.iter().enumerate() {
                if i > 0 {
                    toml.push_str(", ");
                }
                let s = p.as_str().unwrap_or("");
                toml.push_str(&toml_quote(s));
            }
            toml.push_str("]\n");
        }

        toml
    }

    async fn action_create(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = match Self::extract_name(args) {
            Ok(n) => n,
            Err(r) => return Ok(r),
        };
        let skill_path = self.skills_dir.join(&name);
        if skill_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Skill '{name}' already exists at {}",
                    skill_path.display()
                )),
            });
        }
        if let Err(e) = fs::create_dir_all(&skill_path) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to create skill directory: {e}")),
            });
        }
        // Write SKILL.md if content provided, otherwise SKILL.toml
        if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
            if let Err(e) = fs::write(skill_path.join("SKILL.md"), content) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to write SKILL.md: {e}")),
                });
            }
        } else {
            let toml_content = Self::build_toml(&name, args);
            if let Err(e) = fs::write(skill_path.join("SKILL.toml"), &toml_content) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to write SKILL.toml: {e}")),
                });
            }
        }
        // Reload skills into shared state
        self.reload_shared_state().await;
        Ok(ToolResult {
            success: true,
            output: json!({
                "created": name,
                "path": skill_path.display().to_string()
            })
            .to_string(),
            error: None,
        })
    }
    async fn action_read(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = match Self::extract_name(args) {
            Ok(n) => n,
            Err(r) => return Ok(r),
        };
        let skill_path = self.skills_dir.join(&name);
        if !skill_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Skill '{name}' not found")),
            });
        }
        // Try SKILL.toml first, then SKILL.md
        let toml_path = skill_path.join("SKILL.toml");
        let md_path = skill_path.join("SKILL.md");
        if toml_path.exists() {
            match fs::read_to_string(&toml_path) {
                Ok(content) => Ok(ToolResult {
                    success: true,
                    output: json!({
                        "name": name,
                        "format": "toml",
                        "content": content
                    })
                    .to_string(),
                    error: None,
                }),
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to read SKILL.toml: {e}")),
                }),
            }
        } else if md_path.exists() {
            match fs::read_to_string(&md_path) {
                Ok(content) => Ok(ToolResult {
                    success: true,
                    output: json!({
                        "name": name,
                        "format": "md",
                        "content": content
                    })
                    .to_string(),
                    error: None,
                }),
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to read SKILL.md: {e}")),
                }),
            }
        } else {
            Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Skill '{name}' has no SKILL.toml or SKILL.md")),
            })
        }
    }
    async fn action_update(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = match Self::extract_name(args) {
            Ok(n) => n,
            Err(r) => return Ok(r),
        };
        let skill_path = self.skills_dir.join(&name);
        if !skill_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Skill '{name}' not found")),
            });
        }
        // Write SKILL.md if content provided, otherwise SKILL.toml
        if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
            if let Err(e) = fs::write(skill_path.join("SKILL.md"), content) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to write SKILL.md: {e}")),
                });
            }
        } else {
            let toml_content = Self::build_toml(&name, args);
            if let Err(e) = fs::write(skill_path.join("SKILL.toml"), &toml_content) {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to write SKILL.toml: {e}")),
                });
            }
        }
        self.reload_shared_state().await;
        Ok(ToolResult {
            success: true,
            output: json!({ "updated": name }).to_string(),
            error: None,
        })
    }
    async fn action_delete(&self, args: &serde_json::Value) -> anyhow::Result<ToolResult> {
        let name = match Self::extract_name(args) {
            Ok(n) => n,
            Err(r) => return Ok(r),
        };
        let skill_path = self.skills_dir.join(&name);
        if !skill_path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Skill '{name}' not found")),
            });
        }
        // Canonicalize and verify the path is inside skills_dir (symlink escape prevention)
        let canonical = match fs::canonicalize(&skill_path) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to resolve skill path: {e}")),
                });
            }
        };
        let canonical_base = match fs::canonicalize(&self.skills_dir) {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to resolve skills directory: {e}")),
                });
            }
        };
        if !canonical.starts_with(&canonical_base) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Skill path escapes the skills directory".into()),
            });
        }
        if let Err(e) = fs::remove_dir_all(&canonical) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to delete skill directory: {e}")),
            });
        }
        self.reload_shared_state().await;
        Ok(ToolResult {
            success: true,
            output: json!({ "deleted": name }).to_string(),
            error: None,
        })
    }
    async fn action_list(&self) -> anyhow::Result<ToolResult> {
        let state = self.shared_state.read().await;
        let skills: Vec<serde_json::Value> = state
            .skills
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "description": s.description,
                    "version": s.version
                })
            })
            .collect();
        Ok(ToolResult {
            success: true,
            output: json!({ "skills": skills }).to_string(),
            error: None,
        })
    }
    /// Reload skills from disk into shared state.
    async fn reload_shared_state(&self) {
        let mut state = self.shared_state.write().await;
        crate::skills::reload_skills(&mut state, &self.workspace_dir, &self.config);
        state
            .dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Escape and quote a string for TOML basic string format.
fn toml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::skills::SkillsState;
    use serde_json::json;

    /// Create a test tool with a temp directory.
    fn test_tool(dir: &std::path::Path) -> SkillManageTool {
        let skills_dir = dir.join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let config = Arc::new(Config::default());
        let state = Arc::new(RwLock::new(SkillsState::new()));
        SkillManageTool::new(skills_dir, state, dir.to_path_buf(), config)
    }

    #[test]
    fn validate_name_accepts_valid_names() {
        assert!(validate_skill_name("my-skill").is_ok());
        assert!(validate_skill_name("skill_v2").is_ok());
        assert!(validate_skill_name("A").is_ok());
        assert!(validate_skill_name("a1b2c3").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty_and_long() {
        assert!(validate_skill_name("").is_err());
        let long = "a".repeat(65);
        assert!(validate_skill_name(&long).is_err());
    }
    #[test]
    fn test_create_skill_path_traversal() {
        assert!(validate_skill_name("../../etc/passwd").is_err());
        assert!(validate_skill_name("foo/../bar").is_err());
        assert!(validate_skill_name("foo/bar").is_err());
        assert!(validate_skill_name("foo\\bar").is_err());
    }
    #[test]
    fn test_create_skill_windows_reserved() {
        assert!(validate_skill_name("CON").is_err());
        assert!(validate_skill_name("con").is_err());
        assert!(validate_skill_name("PRN").is_err());
        assert!(validate_skill_name("AUX").is_err());
        assert!(validate_skill_name("NUL").is_err());
        assert!(validate_skill_name("COM1").is_err());
        assert!(validate_skill_name("LPT1").is_err());
    }
    #[tokio::test]
    async fn test_create_skill() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let result = tool
            .execute(json!({
                "action": "create",
                "name": "my-skill",
                "description": "Test skill"
            }))
            .await
            .unwrap();
        assert!(result.success, "create failed: {:?}", result.error);
        let toml_path = dir.path().join("skills/my-skill/SKILL.toml");
        assert!(toml_path.exists());
        let content = fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("my-skill"));
        assert!(content.contains("Test skill"));
    }
    #[tokio::test]
    async fn test_create_skill_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        let r1 = tool
            .execute(json!({ "action": "create", "name": "dup-skill" }))
            .await
            .unwrap();
        assert!(r1.success);
        let r2 = tool
            .execute(json!({ "action": "create", "name": "dup-skill" }))
            .await
            .unwrap();
        assert!(!r2.success);
        assert!(r2.error.as_deref().unwrap_or("").contains("already exists"));
    }
    #[tokio::test]
    async fn test_read_skill() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        tool.execute(json!({
            "action": "create",
            "name": "read-me",
            "description": "Readable skill"
        }))
        .await
        .unwrap();
        let result = tool
            .execute(json!({ "action": "read", "name": "read-me" }))
            .await
            .unwrap();
        assert!(result.success, "read failed: {:?}", result.error);
        assert!(result.output.contains("read-me"));
        assert!(result.output.contains("toml"));
    }
    #[tokio::test]
    async fn test_update_skill() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        tool.execute(json!({
            "action": "create",
            "name": "upd-skill",
            "description": "Original"
        }))
        .await
        .unwrap();
        let result = tool
            .execute(json!({
                "action": "update",
                "name": "upd-skill",
                "description": "Updated desc"
            }))
            .await
            .unwrap();
        assert!(result.success, "update failed: {:?}", result.error);
        let content = fs::read_to_string(dir.path().join("skills/upd-skill/SKILL.toml")).unwrap();
        assert!(content.contains("Updated desc"));
    }
    #[tokio::test]
    async fn test_delete_skill() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        tool.execute(json!({
            "action": "create",
            "name": "del-skill"
        }))
        .await
        .unwrap();
        let skill_dir = dir.path().join("skills/del-skill");
        assert!(skill_dir.exists());
        let result = tool
            .execute(json!({ "action": "delete", "name": "del-skill" }))
            .await
            .unwrap();
        assert!(result.success, "delete failed: {:?}", result.error);
        assert!(!skill_dir.exists());
    }
    #[tokio::test]
    async fn test_list_skills() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        tool.execute(json!({
            "action": "create",
            "name": "skill-a",
            "description": "First"
        }))
        .await
        .unwrap();
        tool.execute(json!({
            "action": "create",
            "name": "skill-b",
            "description": "Second"
        }))
        .await
        .unwrap();
        let result = tool.execute(json!({ "action": "list" })).await.unwrap();
        assert!(result.success, "list failed: {:?}", result.error);
        assert!(result.output.contains("skill-a"));
        assert!(result.output.contains("skill-b"));
    }
    #[tokio::test]
    async fn test_create_skill_toml_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let tool = test_tool(dir.path());
        tool.execute(json!({
            "action": "create",
            "name": "roundtrip",
            "description": "Roundtrip test",
            "version": "1.2.3"
        }))
        .await
        .unwrap();
        // Load via crate::skills::load_skills and verify
        let skills = crate::skills::load_skills(dir.path());
        let found = skills.iter().find(|s| s.name == "roundtrip");
        assert!(found.is_some(), "skill not found via load_skills");
        let skill = found.unwrap();
        assert_eq!(skill.description, "Roundtrip test");
        assert_eq!(skill.version, "1.2.3");
    }
}
