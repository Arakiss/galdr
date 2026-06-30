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
    /// Optional recording policy for the hook hot path.
    pub capture: CaptureConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureConfig {
    /// Tool names that must not be recorded.
    pub deny_tools: Vec<String>,
    /// CWD prefixes that must not be recorded.
    pub deny_cwd_prefixes: Vec<String>,
    /// Optional JSON-character cap for recorded tool responses.
    pub max_response_chars: Option<usize>,
    /// Drop screenshot/base64 image blobs (e.g. Computer Use captures) from the
    /// recorded event. On by default: a screenshot is a large, sensitive image whose
    /// reusable signal is the *action*, not the pixels — keeping it bloats the span
    /// and risks leaking on-screen content. The action fields are always kept.
    #[serde(default = "default_strip_screenshots")]
    pub strip_screenshots: bool,
    /// Keep stripped screenshots as **ephemeral** PNG frames under
    /// `~/.galdr/frames/<rec_id>/` so the authoring step can see the screen and write
    /// better semantic steps for a GUI skill. Off by default — pixels on disk are
    /// opt-in. The frames are never in the span, the skill, or an export; they are
    /// purged when a final skill installs from the recording. Scaffolding to *produce*
    /// the skill, not part of it.
    #[serde(default)]
    pub keep_frames: bool,
}

fn default_strip_screenshots() -> bool {
    true
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            deny_tools: Vec::new(),
            deny_cwd_prefixes: Vec::new(),
            max_response_chars: None,
            strip_screenshots: true,
            keep_frames: false,
        }
    }
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
            capture: CaptureConfig::default(),
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

    /// Loads only the capture policy for the sensor hot path. This deliberately
    /// avoids network endpoint validation so a bad distiller config can never
    /// break recording; malformed config simply disables capture policy.
    pub fn load_capture() -> CaptureConfig {
        let Ok(path) = paths::config_file() else {
            return CaptureConfig::default();
        };
        let Ok(contents) = std::fs::read_to_string(path) else {
            return CaptureConfig::default();
        };
        serde_json::from_str::<Config>(&contents)
            .map(|config| config.capture)
            .unwrap_or_default()
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
        assert!(config.capture.deny_tools.is_empty());
    }
}
