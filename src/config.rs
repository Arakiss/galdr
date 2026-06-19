//! Optional configuration for autonomous distillation, in `~/.galdr/config.json`.
//!
//! Tolerant by design: a missing file yields defaults, and any field left out
//! falls back to its default. The one hard rule is the loopback guarantee — the
//! endpoint must be a loopback host, enforced at load time by
//! [`crate::engine::validate_loopback`]. There is no way to point the distiller at
//! a remote host through config.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

/// Default MLX model: a small, instruction-tuned 4-bit model that runs on-device.
pub const DEFAULT_MODEL: &str = "mlx-community/Qwen3-4B-Instruct-2507-4bit";
/// Default loopback endpoint for an OpenAI-compatible `mlx_lm.server`.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8080";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `mlx-http` | `mlx-subprocess` | `agent`.
    pub engine: String,
    /// Loopback endpoint of the MLX HTTP server.
    pub endpoint: String,
    /// Model identifier.
    pub model: String,
    /// Generation cap.
    pub max_tokens: u32,
    /// Sampling temperature; low by default for deterministic, faithful output.
    pub temperature: f32,
    /// Per-request timeout, in seconds.
    pub timeout_secs: u64,
    /// Per-step budget (chars) for raw payloads embedded in the prompt.
    pub raw_field_char_budget: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: "mlx-http".to_string(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
            model: DEFAULT_MODEL.to_string(),
            max_tokens: 2048,
            temperature: 0.2,
            timeout_secs: 120,
            raw_field_char_budget: 800,
        }
    }
}

impl Config {
    /// Loads the config, falling back to defaults if the file is absent. Always
    /// validates that the endpoint is loopback — a non-loopback host is a hard
    /// error, never silently accepted.
    pub fn load() -> Result<Self> {
        let path = paths::config_file()?;
        let config: Config = match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents)
                .with_context(|| format!("invalid config at {}", path.display()))?,
            Err(_) => Config::default(),
        };
        crate::engine::validate_loopback(&config.endpoint).with_context(|| {
            format!(
                "config endpoint {} is not loopback; the distiller is loopback-only",
                config.endpoint
            )
        })?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_loopback_and_low_temperature() {
        let config = Config::default();
        assert!(crate::engine::validate_loopback(&config.endpoint).is_ok());
        assert_eq!(config.temperature, 0.2);
        assert_eq!(config.engine, "mlx-http");
    }

    #[test]
    fn partial_config_fills_defaults() {
        let json = r#"{ "model": "custom-model" }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.model, "custom-model");
        // Untouched fields keep their defaults.
        assert_eq!(config.endpoint, DEFAULT_ENDPOINT);
        assert_eq!(config.max_tokens, 2048);
    }
}
