//! Distillation engines for autonomous (no-agent-in-the-loop) distillation.
//!
//! Every engine here is **loopback-only**. The HTTP engine talks to a local
//! `mlx_lm.server`; [`validate_loopback`] rejects any non-loopback host, and the
//! HTTP engine re-checks before each request. This is what lets galdr keep its
//! "no external network egress" promise while still offering on-device
//! distillation: the optional traffic never leaves `127.0.0.1`.
//!
//! The HTTP client (`reqwest`) is gated behind the `mlx` feature. A plain
//! `cargo build` has no HTTP engine; `distill --auto` then falls back to the
//! Phase 0 draft. The subprocess engine shells out to `python3 -m
//! mlx_lm.generate` and needs no extra crate.

use anyhow::{Result, bail};

use crate::config::Config;

/// A backend that turns a (system, user) prompt pair into a `SKILL.md`.
pub trait DistillEngine {
    fn distill(&self, system: &str, user: &str) -> Result<String>;
    /// Cheap reachability probe, so the caller can fall back cleanly.
    fn detect(&self) -> bool;
}

/// Which engine to use for autonomous distillation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    MlxHttp,
    MlxSubprocess,
    /// No engine: emit the Phase 0 draft for the agent to finish.
    Agent,
}

impl EngineKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "mlx-http" => Ok(Self::MlxHttp),
            "mlx-subprocess" => Ok(Self::MlxSubprocess),
            "agent" => Ok(Self::Agent),
            other => {
                bail!("unknown engine '{other}' (use mlx-http, mlx-subprocess, or agent)")
            }
        }
    }
}

/// Builds the engine for a kind, or `None` when the kind maps to the draft path
/// (the `agent` kind, or `mlx-http` when the `mlx` feature is not compiled).
pub fn build_engine(kind: EngineKind, cfg: &Config) -> Option<Box<dyn DistillEngine>> {
    match kind {
        EngineKind::Agent => None,
        EngineKind::MlxSubprocess => Some(Box::new(MlxSubprocessEngine::new(cfg))),
        EngineKind::MlxHttp => mlx_http_engine(cfg),
    }
}

#[cfg(feature = "mlx")]
fn mlx_http_engine(cfg: &Config) -> Option<Box<dyn DistillEngine>> {
    Some(Box::new(MlxHttpEngine::new(cfg)))
}

#[cfg(not(feature = "mlx"))]
fn mlx_http_engine(_cfg: &Config) -> Option<Box<dyn DistillEngine>> {
    None
}

/// Accepts only loopback hosts. The single guard that keeps the distiller from
/// ever reaching off the machine.
pub fn validate_loopback(url: &str) -> Result<()> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme.split('/').next().unwrap_or("");
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 literal: [::1]:port
        rest.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };
    match host {
        "127.0.0.1" | "::1" | "localhost" => Ok(()),
        other => {
            bail!("non-loopback host '{other}' is not allowed (the distiller is loopback-only)")
        }
    }
}

/// Engine that shells out to `python3 -m mlx_lm.generate`. No extra crate; needs
/// `mlx_lm` installed in the environment's Python.
pub struct MlxSubprocessEngine {
    model: String,
    max_tokens: u32,
    temperature: f32,
}

impl MlxSubprocessEngine {
    pub fn new(cfg: &Config) -> Self {
        Self {
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
        }
    }
}

impl DistillEngine for MlxSubprocessEngine {
    fn detect(&self) -> bool {
        std::process::Command::new("python3")
            .args(["-c", "import mlx_lm"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn distill(&self, system: &str, user: &str) -> Result<String> {
        use std::io::Write;

        let prompt = format!("{system}\n\n{user}");
        let mut child = std::process::Command::new("python3")
            .args([
                "-m",
                "mlx_lm.generate",
                "--model",
                &self.model,
                "--max-tokens",
                &self.max_tokens.to_string(),
                "--temp",
                &self.temperature.to_string(),
                "--prompt",
                "-",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("could not open mlx_lm stdin"))?
            .write_all(prompt.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!("mlx_lm.generate exited with a failure");
        }
        Ok(extract_generation(&String::from_utf8_lossy(&output.stdout)))
    }
}

/// `mlx_lm.generate` brackets its output with `==========` markers; take the text
/// between them, falling back to the whole stdout if the markers are absent.
fn extract_generation(out: &str) -> String {
    let parts: Vec<&str> = out.split("==========").collect();
    if parts.len() >= 2 {
        parts[1].trim().to_string()
    } else {
        out.trim().to_string()
    }
}

/// Engine that posts to a loopback OpenAI-compatible `mlx_lm.server`.
#[cfg(feature = "mlx")]
pub struct MlxHttpEngine {
    endpoint: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    timeout: std::time::Duration,
}

#[cfg(feature = "mlx")]
impl MlxHttpEngine {
    pub fn new(cfg: &Config) -> Self {
        Self {
            endpoint: cfg.endpoint.clone(),
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            temperature: cfg.temperature,
            timeout: std::time::Duration::from_secs(cfg.timeout_secs),
        }
    }
}

#[cfg(feature = "mlx")]
impl DistillEngine for MlxHttpEngine {
    fn detect(&self) -> bool {
        let Ok(client) = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_millis(800))
            .build()
        else {
            return false;
        };
        client
            .get(format!("{}/v1/models", self.endpoint))
            .send()
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn distill(&self, system: &str, user: &str) -> Result<String> {
        use anyhow::Context;
        // Defensive: a config edit can never push a request off the loopback.
        validate_loopback(&self.endpoint)?;

        let client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .build()?;
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
        });
        let response = client
            .post(format!("{}/v1/chat/completions", self.endpoint))
            .json(&body)
            .send()
            .context("MLX server request failed")?;
        let value: serde_json::Value = response.json().context("invalid MLX server response")?;
        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .context("MLX response had no message content")?;
        Ok(content.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_are_accepted() {
        for url in [
            "http://127.0.0.1:8080",
            "http://localhost:1234/v1",
            "http://[::1]:8080",
            "127.0.0.1:9000",
            "http://user@127.0.0.1:8080",
        ] {
            assert!(validate_loopback(url).is_ok(), "{url} should be loopback");
        }
    }

    #[test]
    fn non_loopback_hosts_are_rejected() {
        for url in [
            "http://evil.com",
            "https://example.com/v1",
            "http://10.0.0.5:8080",
            "http://169.254.169.254/latest",
        ] {
            assert!(validate_loopback(url).is_err(), "{url} should be rejected");
        }
    }

    #[test]
    fn engine_kind_parses_and_rejects() {
        assert_eq!(EngineKind::parse("mlx-http").unwrap(), EngineKind::MlxHttp);
        assert_eq!(
            EngineKind::parse("mlx-subprocess").unwrap(),
            EngineKind::MlxSubprocess
        );
        assert_eq!(EngineKind::parse("agent").unwrap(), EngineKind::Agent);
        assert!(EngineKind::parse("openai").is_err());
    }

    #[test]
    fn agent_kind_builds_no_engine() {
        let cfg = Config::default();
        assert!(build_engine(EngineKind::Agent, &cfg).is_none());
    }

    #[test]
    fn extract_generation_reads_between_markers() {
        let out = "prompt echo\n==========\nthe skill body\n==========\nstats: 10 tok/s";
        assert_eq!(extract_generation(out), "the skill body");
        assert_eq!(extract_generation("no markers here"), "no markers here");
    }
}
