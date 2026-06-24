//! Detects which agent harnesses are present on this system.
//!
//! galdr records the tool calls a harness emits, so knowing *which* harnesses are
//! installed — and whether galdr's sensor is wired into each — is part of its setup
//! and diagnostics story. Detection is read-only: it probes well-known config
//! directories and looks for the harness binary on `PATH`. It never runs them.

use std::path::PathBuf;

use directories::BaseDirs;
use serde::Serialize;

use crate::setup;

/// One detected (or absent) agent harness.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessInfo {
    /// Display name, e.g. "Claude Code".
    pub name: String,
    /// Stable key, e.g. "claude".
    pub key: String,
    /// True if a config directory or the binary was found.
    pub detected: bool,
    /// The harness config directory, if it exists.
    pub config_dir: Option<String>,
    /// Whether the harness binary is on `PATH`.
    pub on_path: bool,
    /// Whether galdr's hook is wired into this harness. `Some` only where galdr
    /// knows how to wire one (today: Claude Code); `None` elsewhere.
    pub galdr_hook: Option<bool>,
    /// Short human note (e.g. hook status).
    pub notes: String,
}

struct Known {
    name: &'static str,
    key: &'static str,
    config: &'static str,
    bin: &'static str,
}

/// The harnesses galdr knows how to recognize. Ordered by how common they are.
const KNOWN: &[Known] = &[
    Known {
        name: "Claude Code",
        key: "claude",
        config: ".claude",
        bin: "claude",
    },
    Known {
        name: "Codex",
        key: "codex",
        config: ".codex",
        bin: "codex",
    },
    Known {
        name: "Cursor",
        key: "cursor",
        config: ".cursor",
        bin: "cursor",
    },
    Known {
        name: "Gemini CLI",
        key: "gemini",
        config: ".gemini",
        bin: "gemini",
    },
    Known {
        name: "Aider",
        key: "aider",
        config: ".aider.conf.yml",
        bin: "aider",
    },
    Known {
        name: "Windsurf",
        key: "windsurf",
        config: ".windsurf",
        bin: "windsurf",
    },
];

/// Probes the system for known harnesses.
pub fn detect() -> Vec<HarnessInfo> {
    let home = BaseDirs::new().map(|b| b.home_dir().to_path_buf());
    KNOWN.iter().map(|k| info_for(k, home.as_ref())).collect()
}

fn info_for(k: &Known, home: Option<&PathBuf>) -> HarnessInfo {
    let config_dir = home
        .map(|h| h.join(k.config))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string());
    let on_path = binary_on_path(k.bin);
    let galdr_hook = if k.key == "claude" {
        setup::claude_hook_configured()
    } else {
        None
    };
    let detected = config_dir.is_some() || on_path;
    let notes = match galdr_hook {
        Some(true) => "galdr sensor wired".to_string(),
        Some(false) if detected => "galdr sensor not wired".to_string(),
        _ => String::new(),
    };
    HarnessInfo {
        name: k.name.to_string(),
        key: k.key.to_string(),
        detected,
        config_dir,
        on_path,
        galdr_hook,
        notes,
    }
}

/// True if an executable file named `bin` is found in any `PATH` directory.
fn binary_on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_every_known_harness() {
        let found = detect();
        assert_eq!(found.len(), KNOWN.len());
        assert!(found.iter().any(|h| h.key == "claude"));
        // Claude Code is the one galdr can wire a hook into, so its flag is Some.
        let claude = found.iter().find(|h| h.key == "claude").unwrap();
        // galdr_hook is Some(true|false) only when a Claude settings file exists;
        // either way the field is well-formed and the entry is present.
        let _ = claude.galdr_hook;
        // Other harnesses never carry a hook flag.
        let codex = found.iter().find(|h| h.key == "codex").unwrap();
        assert!(codex.galdr_hook.is_none());
    }

    #[test]
    fn binary_on_path_finds_a_ubiquitous_binary() {
        // `sh` exists on every unix PATH the tests run on.
        assert!(binary_on_path("sh"));
        assert!(!binary_on_path("definitely-not-a-real-binary-xyzzy"));
    }
}
