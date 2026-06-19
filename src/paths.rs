//! On-disk layout of galdr, all under `~/.galdr/`.
//!
//! ```text
//! ~/.galdr/
//! ├── active                 active-recording flag (JSON), absent = not recording
//! ├── spans/<rec_id>.jsonl   append-only span, one line per tool call
//! └── recordings/<rec_id>.json   metadata written when a recording is closed
//! ```
//!
//! Distilled skills are written elsewhere, under `~/.agents/skills/<name>/`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;

/// The user's home directory.
fn home() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine the home directory")?;
    Ok(base.home_dir().to_path_buf())
}

/// galdr's data root: `~/.galdr`.
pub fn galdr_root() -> Result<PathBuf> {
    Ok(home()?.join(".galdr"))
}

/// Active-recording flag: `~/.galdr/active`.
pub fn active_flag() -> Result<PathBuf> {
    Ok(galdr_root()?.join("active"))
}

/// Spans directory: `~/.galdr/spans`.
pub fn spans_dir() -> Result<PathBuf> {
    Ok(galdr_root()?.join("spans"))
}

/// A recording's span: `~/.galdr/spans/<rec_id>.jsonl`.
pub fn span_file(rec_id: &str) -> Result<PathBuf> {
    Ok(spans_dir()?.join(format!("{rec_id}.jsonl")))
}

/// Recording-metadata directory: `~/.galdr/recordings`.
pub fn recordings_dir() -> Result<PathBuf> {
    Ok(galdr_root()?.join("recordings"))
}

/// A recording's metadata: `~/.galdr/recordings/<rec_id>.json`.
pub fn recording_file(rec_id: &str) -> Result<PathBuf> {
    Ok(recordings_dir()?.join(format!("{rec_id}.json")))
}

/// Creates the data directories if missing. Idempotent.
pub fn ensure_dirs() -> Result<()> {
    std::fs::create_dir_all(spans_dir()?)?;
    std::fs::create_dir_all(recordings_dir()?)?;
    Ok(())
}

/// Skills root: `~/.agents/skills`.
pub fn skills_root() -> Result<PathBuf> {
    Ok(home()?.join(".agents").join("skills"))
}

/// A distilled skill's directory: `~/.agents/skills/<name>`.
pub fn skill_dir(name: &str) -> Result<PathBuf> {
    Ok(skills_root()?.join(name))
}
