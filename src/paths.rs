//! On-disk layout of galdr, all under `~/.galdr/`.
//!
//! ```text
//! ~/.galdr/
//! ├── active                 active-recording flag (JSON), absent = not recording
//! ├── config.json            optional config (distill engine, endpoint, model)
//! ├── galdrd.sock            daemon control socket (NDJSON over a Unix socket)
//! ├── galdrd.pid             daemon pidfile
//! ├── catalog.sqlite         queryable index, rebuilt from spans/ + recordings/
//! ├── spans/<rec_id>.jsonl   append-only span, one line per tool call
//! └── recordings/<rec_id>.json   metadata written when a recording is closed
//! ```
//!
//! Distilled skills are written elsewhere, under `~/.agents/skills/<name>/`.
//!
//! The SQLite catalog is an **index, never the truth**: it can be deleted and
//! rebuilt at any time from the spans and recordings with `galdr reindex`.

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

/// Daemon control socket: `~/.galdr/galdrd.sock`.
pub fn socket_path() -> Result<PathBuf> {
    Ok(galdr_root()?.join("galdrd.sock"))
}

/// Daemon pidfile: `~/.galdr/galdrd.pid`.
pub fn pidfile() -> Result<PathBuf> {
    Ok(galdr_root()?.join("galdrd.pid"))
}

/// Queryable catalog database: `~/.galdr/catalog.sqlite`.
pub fn catalog_db() -> Result<PathBuf> {
    Ok(galdr_root()?.join("catalog.sqlite"))
}

/// Optional config file: `~/.galdr/config.json`.
pub fn config_file() -> Result<PathBuf> {
    Ok(galdr_root()?.join("config.json"))
}

/// Skills root: `~/.agents/skills`.
pub fn skills_root() -> Result<PathBuf> {
    Ok(home()?.join(".agents").join("skills"))
}

/// A distilled skill's directory: `~/.agents/skills/<name>`.
pub fn skill_dir(name: &str) -> Result<PathBuf> {
    Ok(skills_root()?.join(name))
}
