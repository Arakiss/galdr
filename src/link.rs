//! Makes a distilled skill discoverable by the harnesses installed on the system.
//!
//! galdr installs a skill once, in the open-standard skills root (`~/.agents/skills`
//! by default — the same location Codex's Agent Skills use). But each harness loads
//! skills from *its own* directory: Claude Code from `~/.claude/skills`, Codex from
//! `~/.codex/skills`, Cursor from `~/.cursor/skills-cursor`. A skill that only lives
//! in the open-standard root is therefore invisible to the harness it was recorded
//! in — galdr would record and distill, then dead-end at a file nothing loads.
//!
//! This module bridges that gap with a symlink per detected harness, pointing the
//! harness's skills directory at the canonical skill. It is the on-disk mechanism
//! that already works for hand-linked skills (e.g. `~/.claude/skills/orca-cli ->
//! ~/.agents/skills/orca-cli`), made automatic and reversible.

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::{harness, paths};

/// The outcome of linking one skill into one harness.
#[derive(Debug, Clone, Serialize)]
pub struct LinkResult {
    pub harness: String,
    pub skill: String,
    /// Where the harness will now find the skill.
    pub link_path: String,
    pub status: LinkStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    /// A fresh symlink was created.
    Linked,
    /// The link was already present and correct.
    AlreadyLinked,
    /// The harness's skills dir *is* the canonical root — nothing to do.
    SameRoot,
    /// A real file/dir (not our symlink) already occupies the path; left untouched.
    Conflict,
    /// The link could not be created (e.g. permissions).
    Failed,
}

impl LinkStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LinkStatus::Linked => "linked",
            LinkStatus::AlreadyLinked => "already linked",
            LinkStatus::SameRoot => "same root",
            LinkStatus::Conflict => "conflict",
            LinkStatus::Failed => "failed",
        }
    }
}

/// The outcome of unlinking one skill from one harness — the inverse of [`LinkResult`],
/// used when a skill is retired (`galdr rm`).
#[derive(Debug, Clone, Serialize)]
pub struct UnlinkResult {
    pub harness: String,
    pub skill: String,
    /// The harness path that pointed (or would have pointed) at the skill.
    pub link_path: String,
    pub status: UnlinkStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnlinkStatus {
    /// Our symlink to the canonical skill was verified and removed.
    Unlinked,
    /// Nothing was there to remove.
    Absent,
    /// The harness's skills dir *is* the canonical root — the skill lives there, not
    /// as a separate link; retiring the directory itself handles it.
    SameRoot,
    /// A path is present but is not our symlink to *this* skill (a real dir, or a
    /// symlink pointing elsewhere); left untouched.
    Foreign,
    /// The symlink could not be removed (e.g. permissions).
    Failed,
}

/// Links one installed skill into every detected harness whose skills directory
/// galdr knows. Returns one result per harness considered; harnesses that are not
/// installed, or whose skills location is unknown, are skipped silently.
pub fn link_skill(skill_name: &str) -> Result<Vec<LinkResult>> {
    let canonical = paths::skill_dir(skill_name)?;
    let mut results = Vec::new();
    for info in harness::detect() {
        if !info.detected {
            continue;
        }
        let Some(target_dir) = harness::skills_dir(&info.key) else {
            continue; // galdr doesn't know this harness's skills dir
        };
        results.push(link_into(&info.name, skill_name, &canonical, &target_dir));
    }
    Ok(results)
}

/// Re-links skills in the open-standard root into every detected harness. With
/// `all = false` (the default for `galdr link`) only galdr-distilled skills are
/// linked — galdr's job is its own R/R skills, not fanning the user's hand-authored
/// skills across harnesses. With `all = true` every skill in the root is linked, for
/// those who deliberately want galdr to sync the whole open-standard directory.
pub fn link_all(all: bool) -> Result<Vec<LinkResult>> {
    let root = paths::skills_root()?;
    let mut results = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(results);
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                return None;
            }
            if !all
                && crate::catalog::skill_origin(&skill_md.to_string_lossy())
                    != crate::catalog::ORIGIN_GALDR
            {
                return None;
            }
            path.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .collect();
    names.sort();
    for name in names {
        results.extend(link_skill(&name)?);
    }
    Ok(results)
}

/// Removes one installed skill's symlink from every detected harness. The inverse of
/// [`link_skill`]: for each harness it removes the symlink only when it truly points at
/// this skill's canonical directory, never a real file or a link the user aimed
/// elsewhere. Returns one result per harness considered.
pub fn unlink_skill(skill_name: &str) -> Result<Vec<UnlinkResult>> {
    let canonical = paths::skill_dir(skill_name)?;
    let mut results = Vec::new();
    for info in harness::detect() {
        if !info.detected {
            continue;
        }
        let Some(target_dir) = harness::skills_dir(&info.key) else {
            continue; // galdr doesn't know this harness's skills dir
        };
        results.push(unlink_from(&info.name, skill_name, &canonical, &target_dir));
    }
    Ok(results)
}

fn unlink_from(
    harness_name: &str,
    skill: &str,
    canonical: &Path,
    target_dir: &Path,
) -> UnlinkResult {
    let link_path = target_dir.join(skill);
    let mk = |status| UnlinkResult {
        harness: harness_name.to_string(),
        skill: skill.to_string(),
        link_path: link_path.display().to_string(),
        status,
    };

    // The harness loads from the canonical root itself: there is no separate link to
    // remove — retiring the skill directory is what takes it away here.
    if same_dir(target_dir, canonical.parent().unwrap_or(canonical)) {
        return mk(UnlinkStatus::SameRoot);
    }

    match std::fs::symlink_metadata(&link_path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Only remove a symlink we can prove points at *this* skill's canonical dir;
            // a link the user aimed elsewhere is not ours to touch.
            match std::fs::read_link(&link_path) {
                Ok(dest) if same_dir(&dest, canonical) => match std::fs::remove_file(&link_path) {
                    Ok(()) => mk(UnlinkStatus::Unlinked),
                    Err(_) => mk(UnlinkStatus::Failed),
                },
                _ => mk(UnlinkStatus::Foreign),
            }
        }
        // A real directory or file is there — never the symlink we created.
        Ok(_) => mk(UnlinkStatus::Foreign),
        // Nothing there.
        Err(_) => mk(UnlinkStatus::Absent),
    }
}

fn link_into(harness_name: &str, skill: &str, canonical: &Path, target_dir: &Path) -> LinkResult {
    let link_path = target_dir.join(skill);
    let mk = |status| LinkResult {
        harness: harness_name.to_string(),
        skill: skill.to_string(),
        link_path: link_path.display().to_string(),
        status,
    };

    // The harness loads from the canonical root itself: the skill is already there.
    if same_dir(target_dir, canonical.parent().unwrap_or(canonical)) {
        return mk(LinkStatus::SameRoot);
    }

    match std::fs::symlink_metadata(&link_path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Our link already? Point it at the canonical dir if it drifted.
            match std::fs::read_link(&link_path) {
                Ok(dest) if same_dir(&dest, canonical) => mk(LinkStatus::AlreadyLinked),
                _ => {
                    let _ = std::fs::remove_file(&link_path);
                    create(canonical, &link_path, mk)
                }
            }
        }
        // A real directory or file is already there — never clobber the user's own.
        Ok(_) => mk(LinkStatus::Conflict),
        // Nothing there: create the link (making the harness skills dir if needed).
        Err(_) => create(canonical, &link_path, mk),
    }
}

fn create(canonical: &Path, link_path: &Path, mk: impl Fn(LinkStatus) -> LinkResult) -> LinkResult {
    if let Some(parent) = link_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match symlink(canonical, link_path) {
        Ok(()) => mk(LinkStatus::Linked),
        Err(_) => mk(LinkStatus::Failed),
    }
}

/// Compares two directory paths by their canonicalized form when possible, falling
/// back to a literal comparison so the check still works for not-yet-created paths.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => normalize(a) == normalize(b),
    }
}

fn normalize(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_status_strings_are_stable() {
        assert_eq!(LinkStatus::Linked.as_str(), "linked");
        assert_eq!(LinkStatus::Conflict.as_str(), "conflict");
    }

    #[test]
    fn same_dir_ignores_a_trailing_slash() {
        assert!(same_dir(Path::new("/a/b"), Path::new("/a/b/")));
        assert!(!same_dir(Path::new("/a/b"), Path::new("/a/c")));
    }
}
