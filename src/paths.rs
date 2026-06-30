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
//! ├── outcomes/*.jsonl       append-only skill usage and outcome labels
//! └── recordings/<rec_id>.json   metadata written when a recording is closed
//! ```
//!
//! Distilled skills are written elsewhere, under `~/.agents/skills/<name>/`.
//!
//! The SQLite catalog is an **index, never the truth**: it can be deleted and
//! rebuilt at any time from the spans and recordings with `galdr reindex`.
//!
//! The root is `~/.galdr` by default but can be relocated with the `GALDR_ROOT`
//! environment variable (and the skills root with `GALDR_SKILLS_ROOT`), which is
//! what makes hermetic tests, profiles, and CI possible without hijacking `$HOME`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;

/// The user's home directory.
fn home() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine the home directory")?;
    Ok(base.home_dir().to_path_buf())
}

/// The user's home directory, or `None` if it cannot be determined. Used by the
/// validation gate to generalize a recorded absolute path (`/Users/<n>/…`) back to
/// `~/…` so a personal path never lands in an installed, shareable `SKILL.md`.
pub fn home_dir() -> Option<PathBuf> {
    home().ok()
}

/// Reads a directory override from the environment, ignoring an empty value so an
/// accidental `GALDR_ROOT=` never points the root at the filesystem root.
fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// galdr's data root: `$GALDR_ROOT` if set, else `~/.galdr`.
///
/// The override is what makes hermetic tests, throwaway profiles, and CI possible
/// without hijacking the whole `$HOME`. It also gives a way out of the Unix-socket
/// path-length limit (`SUN_LEN`): point the root somewhere short.
pub fn galdr_root() -> Result<PathBuf> {
    if let Some(root) = env_dir("GALDR_ROOT") {
        return Ok(root);
    }
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

/// Ephemeral authoring frames root: `~/.galdr/frames`. Opt-in (`capture.keep_frames`),
/// never part of the span or a skill, purged when a final skill installs.
pub fn frames_root() -> Result<PathBuf> {
    Ok(galdr_root()?.join("frames"))
}

/// A recording's ephemeral frames: `~/.galdr/frames/<rec_id>`.
pub fn frames_dir(rec_id: &str) -> Result<PathBuf> {
    Ok(frames_root()?.join(rec_id))
}

/// Skill usage and outcome-label directory: `~/.galdr/outcomes`.
pub fn outcomes_dir() -> Result<PathBuf> {
    Ok(galdr_root()?.join("outcomes"))
}

/// Append-only skill usage log: `~/.galdr/outcomes/skill_usage.jsonl`.
pub fn skill_usage_log() -> Result<PathBuf> {
    Ok(outcomes_dir()?.join("skill_usage.jsonl"))
}

/// Append-only skill outcome-label log: `~/.galdr/outcomes/skill_outcomes.jsonl`.
pub fn skill_outcomes_log() -> Result<PathBuf> {
    Ok(outcomes_dir()?.join("skill_outcomes.jsonl"))
}

/// A recording's metadata: `~/.galdr/recordings/<rec_id>.json`.
pub fn recording_file(rec_id: &str) -> Result<PathBuf> {
    Ok(recordings_dir()?.join(format!("{rec_id}.json")))
}

/// Creates the data directories if missing. Idempotent.
///
/// The root is locked to `0700`: spans hold raw `tool_input`/`tool_response`, which
/// may contain secrets, and the catalog reveals project paths. Another local user
/// must not be able to read them, so we tighten the root every time (cheap, and it
/// repairs a root that predates this hardening or was created with a loose umask).
pub fn ensure_dirs() -> Result<()> {
    let root = galdr_root()?;
    std::fs::create_dir_all(&root)?;
    restrict_to_owner(&root);
    std::fs::create_dir_all(spans_dir()?)?;
    std::fs::create_dir_all(recordings_dir()?)?;
    std::fs::create_dir_all(outcomes_dir()?)?;
    Ok(())
}

/// Best-effort `chmod 0700` on a path we own. Failure is non-fatal: tightening
/// permissions must never block recording. No-op on platforms without Unix perms.
fn restrict_to_owner(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = path;
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

/// Claude Code settings file inspected by `galdr setup claude`.
pub fn claude_settings() -> Result<PathBuf> {
    Ok(home()?.join(".claude").join("settings.json"))
}

/// Codex hooks file inspected by `galdr setup codex`. Codex uses the same hook
/// shape as Claude Code, in its own `~/.codex/hooks.json`.
pub fn codex_hooks() -> Result<PathBuf> {
    Ok(home()?.join(".codex").join("hooks.json"))
}

/// Skills root: `$GALDR_SKILLS_ROOT` if set, else `~/.agents/skills`.
pub fn skills_root() -> Result<PathBuf> {
    if let Some(root) = env_dir("GALDR_SKILLS_ROOT") {
        return Ok(root);
    }
    Ok(home()?.join(".agents").join("skills"))
}

/// A distilled skill's directory: `~/.agents/skills/<name>`.
///
/// The name must be a single, safe path component. Recording-derived names are
/// already safe (slugify strips everything but alphanumerics and dashes), but
/// `galdr link --skill <name>` and `galdr outcome --skill <name>` take the name
/// raw — without this guard, `--skill ../../x` would escape the skills root and let
/// galdr create or follow a symlink anywhere the user can write.
pub fn skill_dir(name: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;
    Ok(skills_root()?.join(name))
}

/// Refuses a skill directory that already exists as a symlink, so a subsequent
/// `create_dir_all` / write cannot follow it to clobber a file outside the skills
/// root. galdr is the only writer of these directories; a symlink there is not ours.
pub fn ensure_not_symlinked(dir: &std::path::Path) -> Result<()> {
    use anyhow::bail;
    if let Ok(meta) = std::fs::symlink_metadata(dir)
        && meta.file_type().is_symlink()
    {
        bail!(
            "skill directory {} is a symlink; refusing to write through it",
            dir.display()
        );
    }
    Ok(())
}

/// Rejects a skill name that is not a single safe path component.
fn validate_skill_name(name: &str) -> Result<()> {
    use anyhow::bail;
    if name.is_empty() {
        bail!("skill name cannot be empty");
    }
    if name == "." || name == ".." {
        bail!("invalid skill name '{name}'");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("skill name '{name}' must not contain a path separator");
    }
    if name.contains('\0') || name.chars().any(|c| c.is_control()) {
        bail!("skill name contains a control character");
    }
    Ok(())
}
