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
//! Phase 0 draft.

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
    /// No engine: emit the Phase 0 draft for the agent to finish.
    Agent,
}

impl EngineKind {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "mlx-http" => Ok(Self::MlxHttp),
            "agent" => Ok(Self::Agent),
            other => {
                bail!("unknown engine '{other}' (use mlx-http or agent)")
            }
        }
    }
}

/// Builds the engine for a kind, or `None` when the kind maps to the draft path
/// (the `agent` kind, or `mlx-http` when the `mlx` feature is not compiled).
pub fn build_engine(kind: EngineKind, cfg: &Config) -> Option<Box<dyn DistillEngine>> {
    match kind {
        EngineKind::Agent => None,
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
///
/// Parsed defensively: the authority ends at the FIRST `/`, `?`, or `#`, so tricks
/// like `http://evil.com#@127.0.0.1` (where a naive `rsplit('@')` would see the
/// loopback) resolve to the real host `evil.com` and are rejected. The host is then
/// matched by `IpAddr::is_loopback()` (covering all of 127.0.0.0/8 and ::1), or the
/// literal `localhost`.
pub fn validate_loopback(url: &str) -> Result<()> {
    use std::net::IpAddr;

    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    // The authority is everything before the first path/query/fragment delimiter.
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    // Userinfo (anything before the last '@') is stripped; the host is what remains.
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 literal: [::1]:port
        rest.split(']').next().unwrap_or("")
    } else {
        hostport.split(':').next().unwrap_or("")
    };

    if host == "localhost" {
        return Ok(());
    }
    if let Ok(ip) = host.parse::<IpAddr>()
        && ip.is_loopback()
    {
        return Ok(());
    }
    bail!("non-loopback host '{host}' is not allowed (the distiller is loopback-only)")
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
            // A loopback endpoint could still 30x-redirect to an off-host address;
            // forbidding redirects keeps the "no external egress" promise intact.
            .redirect(reqwest::redirect::Policy::none())
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
            .redirect(reqwest::redirect::Policy::none())
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
    fn fragment_and_query_userinfo_tricks_are_rejected() {
        // The authority ends at the first / ? #, so these resolve to the real
        // off-host host, not the loopback hidden after an @.
        for url in [
            "http://evil.com#@127.0.0.1",
            "http://evil.com?@127.0.0.1",
            "http://evil.com/@127.0.0.1",
            "http://127.0.0.1.evil.com:8080",
            "http://0.0.0.0:8080",
        ] {
            assert!(
                validate_loopback(url).is_err(),
                "{url} must not be accepted as loopback"
            );
        }
    }

    #[test]
    fn loopback_range_is_accepted() {
        // is_loopback() covers all of 127.0.0.0/8, not just 127.0.0.1.
        assert!(validate_loopback("http://127.0.0.2:8080").is_ok());
    }

    #[test]
    fn engine_kind_parses_and_rejects() {
        assert_eq!(EngineKind::parse("mlx-http").unwrap(), EngineKind::MlxHttp);
        assert_eq!(EngineKind::parse("agent").unwrap(), EngineKind::Agent);
        assert!(EngineKind::parse("openai").is_err());
        assert!(EngineKind::parse("mlx-subprocess").is_err());
    }

    #[test]
    fn agent_kind_builds_no_engine() {
        let cfg = Config::default();
        assert!(build_engine(EngineKind::Agent, &cfg).is_none());
    }
}
