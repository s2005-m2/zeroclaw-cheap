//! External provider registry — runtime-loadable provider definitions via TOML files.
//!
//! Users place `*.toml` files in `~/.zeroclaw/providers/` to register custom providers.
//! Each file declares a provider name, protocol, base URL, and optional credentials.
//! The registry merges these at startup and supports hot-reload via the gateway API.

use crate::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
use crate::providers::traits::Provider;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

// ── TOML schema ────────────────────────────────────────────────────

/// Supported protocol types for external providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderProtocol {
    OpenaiCompatible,
    AnthropicCompatible,
    GeminiCompatible,
}

/// Authentication style for the provider API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalAuthStyle {
    #[default]
    Bearer,
    XApiKey,
}

/// A single external provider definition, parsed from a TOML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalProviderConfig {
    /// Unique provider name (used in config.toml `default_provider`, delegate `provider`, etc.)
    pub name: String,
    /// Protocol compatibility type.
    pub protocol: ProviderProtocol,
    /// Base URL for the provider API (e.g. `https://my-api.com/v1`).
    pub base_url: String,
    /// Optional API key. If omitted, falls back to caller-supplied credential.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Environment variable name to read API key from (takes precedence over `api_key`).
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Authentication style. Defaults to `bearer`.
    #[serde(default)]
    pub auth_style: ExternalAuthStyle,
    /// Whether the provider supports vision/image inputs.
    #[serde(default)]
    pub vision: bool,
    /// Extra parameters passed through to the provider.
    /// Recognised keys (protocol-dependent):
    /// - `openai-compatible`: `merge_system_into_user` (bool), `supports_responses_fallback` (bool), `user_agent` (string)
    #[serde(default)]
    pub extra: HashMap<String, toml::Value>,
}

impl ExternalProviderConfig {
    /// Validate the config after parsing.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.is_empty() {
            bail!("'name' must not be empty");
        }
        if self.name.contains(':') || self.name.contains(' ') {
            bail!("'name' must not contain ':' or spaces: '{}'", self.name);
        }
        if self.base_url.is_empty() {
            bail!("'base_url' must not be empty for provider '{}'", self.name);
        }
        let url_lower = self.base_url.to_lowercase();
        if !url_lower.starts_with("http://") && !url_lower.starts_with("https://") {
            bail!(
                "'base_url' must start with http:// or https:// for provider '{}': '{}'",
                self.name,
                self.base_url
            );
        }
        Ok(())
    }
    /// Resolve the effective API key: env var > inline > caller fallback.
    pub fn resolve_credential(&self, fallback: Option<&str>) -> Option<String> {
        if let Some(env_var) = &self.api_key_env {
            if let Ok(val) = std::env::var(env_var) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
        self.api_key
            .clone()
            .or_else(|| fallback.map(ToString::to_string))
    }
}

// ── Helpers for extra config ──────────────────────────────────
fn extra_bool(extra: &HashMap<String, toml::Value>, key: &str, default: bool) -> bool {
    extra.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}
fn extra_str(extra: &HashMap<String, toml::Value>, key: &str) -> Option<String> {
    extra.get(key).and_then(|v| v.as_str()).map(ToString::to_string)
}

// ── Provider factory from config ────────────────────────────────
/// Create a `Box<dyn Provider>` from an external provider config.
pub fn create_external_provider(
    config: &ExternalProviderConfig,
    caller_credential: Option<&str>,
) -> anyhow::Result<Box<dyn Provider>> {
    let credential = config.resolve_credential(caller_credential);
    let key = credential.as_deref();
    match config.protocol {
        ProviderProtocol::OpenaiCompatible => {
            let auth = match config.auth_style {
                ExternalAuthStyle::Bearer => AuthStyle::Bearer,
                ExternalAuthStyle::XApiKey => AuthStyle::XApiKey,
            };
            let merge_system = extra_bool(&config.extra, "merge_system_into_user", false);
            let responses_fallback = extra_bool(&config.extra, "supports_responses_fallback", true);
            let user_agent = extra_str(&config.extra, "user_agent");
            Ok(Box::new(OpenAiCompatibleProvider::new_with_options(
                &config.name,
                &config.base_url,
                key,
                auth,
                config.vision,
                responses_fallback,
                user_agent.as_deref(),
                merge_system,
            )))
        }
        ProviderProtocol::AnthropicCompatible => {
            Ok(Box::new(
                crate::providers::anthropic::AnthropicProvider::with_base_url(
                    key,
                    Some(&config.base_url),
                ),
            ))
        }
        ProviderProtocol::GeminiCompatible => {
            Ok(Box::new(
                crate::providers::gemini::GeminiProvider::new_api_key_only(
                    key,
                    &config.base_url,
                ),
            ))
        }
    }
}

// ── Built-in provider names (reserved, cannot be overridden) ────
fn builtin_provider_names() -> HashSet<&'static str> {
    HashSet::from([
        "openrouter", "anthropic", "openai", "ollama",
        "gemini", "google", "google-gemini", "telnyx",
        "venice", "vercel", "vercel-ai",
        "cloudflare", "cloudflare-ai",
        "moonshot", "moonshot-cn",
        "kimi-code", "kimi_coding", "kimi_for_coding",
        "synthetic", "opencode", "opencode-zen",
        "bedrock", "aws-bedrock",
        "groq", "mistral", "xai", "grok",
        "deepseek", "together", "together-ai",
        "fireworks", "fireworks-ai",
        "novita", "perplexity", "cohere",
        "copilot", "github-copilot",
        "lmstudio", "lm-studio",
        "llamacpp", "llama.cpp",
        "sglang", "vllm", "osaurus",
        "nvidia", "nvidia-nim", "build.nvidia.com",
        "astrai", "ovhcloud", "ovh",
        "openai-codex", "openai_codex", "codex",
    ])
}

// ── ProviderRegistry ────────────────────────────────────────────
/// Result of a reload operation.
#[derive(Debug, Clone, Serialize)]
pub struct ReloadResult {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub errors: Vec<String>,
}
/// Thread-safe registry for external (user-defined) providers.
pub struct ProviderRegistry {
    external: RwLock<HashMap<String, ExternalProviderConfig>>,
    reserved: HashSet<&'static str>,
    providers_dir: PathBuf,
}
impl ProviderRegistry {
    pub fn new(providers_dir: PathBuf) -> Self {
        Self {
            external: RwLock::new(HashMap::new()),
            reserved: builtin_provider_names(),
            providers_dir,
        }
    }
    fn is_reserved(&self, name: &str) -> bool {
        self.reserved.contains(name)
            || name.starts_with("custom:")
            || name.starts_with("anthropic-custom:")
    }
    /// Try to create a provider from the external registry.
    pub fn try_create(
        &self,
        name: &str,
        caller_credential: Option<&str>,
    ) -> Option<anyhow::Result<Box<dyn Provider>>> {
        let external = self.external.read().expect("registry lock poisoned");
        let config = external.get(name)?;
        Some(create_external_provider(config, caller_credential))
    }
    /// Scan the providers directory and reload all external providers.
    pub fn reload(&self) -> ReloadResult {
        let mut result = ReloadResult {
            added: Vec::new(),
            removed: Vec::new(),
            errors: Vec::new(),
        };
        let new_configs = self.scan_provider_files(&mut result.errors);
        let mut valid: HashMap<String, ExternalProviderConfig> = HashMap::new();
        for (name, config) in new_configs {
            if self.is_reserved(&name) {
                result.errors.push(format!(
                    "Provider '{}' conflicts with built-in provider name",
                    name
                ));
                continue;
            }
            valid.insert(name, config);
        }
        let mut external = self.external.write().expect("registry lock poisoned");
        let old_names: HashSet<String> = external.keys().cloned().collect();
        let new_names: HashSet<String> = valid.keys().cloned().collect();
        for name in new_names.difference(&old_names) {
            result.added.push(name.clone());
        }
        for name in old_names.difference(&new_names) {
            result.removed.push(name.clone());
        }
        *external = valid;
        result.added.sort();
        result.removed.sort();
        result
    }
    fn scan_provider_files(
        &self,
        errors: &mut Vec<String>,
    ) -> HashMap<String, ExternalProviderConfig> {
        let mut configs: HashMap<String, (PathBuf, ExternalProviderConfig)> = HashMap::new();
        let dir = &self.providers_dir;
        if !dir.is_dir() {
            return configs.into_iter().map(|(k, (_, v))| (k, v)).collect();
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) => {
                errors.push(format!(
                    "Failed to read providers directory '{}': {err}",
                    dir.display()
                ));
                return configs.into_iter().map(|(k, (_, v))| (k, v)).collect();
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match self.parse_provider_file(&path) {
                Ok(config) => {
                    if let Some((first_path, _)) = configs.get(&config.name) {
                        errors.push(format!(
                            "Duplicate provider name '{}': first defined in '{}', redefined in '{}'",
                            config.name,
                            first_path.display(),
                            path.display()
                        ));
                    } else {
                        configs.insert(config.name.clone(), (path, config));
                    }
                }
                Err(err) => {
                    errors.push(format!(
                        "Failed to parse '{}': {err}",
                        path.display()
                    ));
                }
            }
        }
        configs.into_iter().map(|(k, (_, v))| (k, v)).collect()
    }
    fn parse_provider_file(&self, path: &Path) -> anyhow::Result<ExternalProviderConfig> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading '{}'", path.display()))?;
        let config: ExternalProviderConfig = toml::from_str(&content)
            .with_context(|| format!("parsing '{}'", path.display()))?;
        config.validate()?;
        Ok(config)
    }
}
