//! Retiring a skill: `galdr rm <skill>`.
//!
//! There was no CLI way to take a distilled skill out of circulation — you had to
//! `mv` its directory by hand, and any harness symlinks `galdr link` created were left
//! dangling. This retires a skill cleanly and reversibly: unlink it from every harness
//! (only the symlinks that truly point at it), move its directory into the local
//! `~/.agents/skills/.retired/` convention (never a hard delete), then refresh the
//! catalog so it drops out of `galdr skills`.
//!
//! [`retire_skill_dir`] is the shared mechanism, reused by `distill --from` when a
//! frontmatter rename leaves a stale recording-slug directory to clear away.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{link, paths};

/// The local convention for retired skills, a sibling of the active skills.
const RETIRED_DIR: &str = ".retired";

/// What a retirement did: which harness links it removed, and where the skill went.
pub struct RetireOutcome {
    pub unlinked: Vec<link::UnlinkResult>,
    pub moved_to: PathBuf,
}

/// `galdr rm <skill>`: retire an installed skill. Refuses a name that is not
/// installed (exit non-zero). Without `--force` it asks for confirmation at a TTY and
/// requires `--force` when there is none, so a scripted run can never retire a skill
/// by surprise.
pub fn run(skill_name: &str, force: bool) -> Result<()> {
    // `skill_dir` validates the name (no path traversal) before we act on it.
    let canonical = paths::skill_dir(skill_name)?;
    if !canonical.is_dir() {
        bail!(
            "skill '{skill_name}' is not installed under {}",
            tilde(&paths::skills_root()?)
        );
    }

    if !force && !confirm(skill_name)? {
        println!("aborted; '{skill_name}' was not retired.");
        return Ok(());
    }

    let outcome = retire_skill_dir(skill_name)?;
    refresh_catalog();

    let unlinked: Vec<&link::UnlinkResult> = outcome
        .unlinked
        .iter()
        .filter(|r| r.status == link::UnlinkStatus::Unlinked)
        .collect();
    if unlinked.is_empty() {
        println!("Unlinked from no harness (none held a link to it).");
    } else {
        println!("Unlinked from {} harness(es):", unlinked.len());
        for r in unlinked {
            println!("  {} → {}", r.harness, r.link_path);
        }
    }
    for r in &outcome.unlinked {
        if r.status == link::UnlinkStatus::Failed {
            eprintln!(
                "warning: could not remove the {} link at {}",
                r.harness, r.link_path
            );
        }
    }
    println!(
        "{} retired '{skill_name}' → {}",
        crate::style::green("✓"),
        tilde(&outcome.moved_to)
    );
    Ok(())
}

/// Retires a skill directory: unlink its harness symlinks (only those pointing at it),
/// then move the canonical directory into `~/.agents/skills/.retired/<name>`. Never a
/// hard delete; a name already present under `.retired` is kept with a numeric suffix
/// so an earlier retirement is not clobbered. The caller must have verified the
/// directory exists.
pub fn retire_skill_dir(skill_name: &str) -> Result<RetireOutcome> {
    // Unlink first, while the canonical directory still exists, so each symlink's target
    // can be verified against it; then move the directory out of the active root.
    let unlinked = link::unlink_skill(skill_name)?;

    let canonical = paths::skill_dir(skill_name)?;
    let retired_root = paths::skills_root()?.join(RETIRED_DIR);
    std::fs::create_dir_all(&retired_root)
        .with_context(|| format!("could not create {}", retired_root.display()))?;
    let dest = retired_dest(&retired_root, skill_name);
    std::fs::rename(&canonical, &dest).with_context(|| {
        format!(
            "could not move {} to {}",
            canonical.display(),
            dest.display()
        )
    })?;
    Ok(RetireOutcome {
        unlinked,
        moved_to: dest,
    })
}

/// A free destination under `.retired` for `skill_name`: the bare name if it is
/// unused, otherwise `<name>.1`, `<name>.2`, … so a prior retirement of the same name
/// is preserved rather than overwritten.
fn retired_dest(retired_root: &Path, skill_name: &str) -> PathBuf {
    let base = retired_root.join(skill_name);
    if !base.exists() {
        return base;
    }
    let mut n = 1u32;
    loop {
        let candidate = retired_root.join(format!("{skill_name}.{n}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Asks the operator to confirm a retirement. At a TTY it prompts and reads a line;
/// with no TTY it refuses outright, since a scripted run must opt in explicitly with
/// `--force`. Returns whether to proceed.
fn confirm(skill_name: &str) -> Result<bool> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        bail!(
            "refusing to retire '{skill_name}' without confirmation; re-run with --force in a non-interactive context"
        );
    }
    print!(
        "Retire skill '{skill_name}'? It is unlinked from every harness and moved to {}. [y/N] ",
        tilde(&paths::skills_root()?.join(RETIRED_DIR))
    );
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Rebuilds the catalog after a retirement so the skill drops out of `galdr skills`
/// (its directory no longer sits directly under the active root). Prefers the running
/// daemon, falls back to a local reindex; best-effort, since the on-disk move is the
/// source of truth either way.
fn refresh_catalog() {
    if matches!(
        crate::ipc::query(&crate::ipc::Request::Reindex),
        Ok(crate::ipc::Response::Reindexed { .. })
    ) {
        return;
    }
    if let Ok(mut conn) = crate::catalog::open() {
        let _ = crate::catalog::reindex(&mut conn);
    }
}

/// Abbreviates the home directory to `~` for friendlier, shareable output.
fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match paths::home_dir() {
        Some(home) => shown
            .strip_prefix(&home.display().to_string())
            .map(|rest| format!("~{rest}"))
            .unwrap_or(shown),
        None => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_dest_suffixes_a_name_already_present() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Nothing there yet: the bare name.
        assert_eq!(retired_dest(root, "demo"), root.join("demo"));
        // Occupy it, then the next free suffix is picked.
        std::fs::create_dir_all(root.join("demo")).unwrap();
        assert_eq!(retired_dest(root, "demo"), root.join("demo.1"));
        std::fs::create_dir_all(root.join("demo.1")).unwrap();
        assert_eq!(retired_dest(root, "demo"), root.join("demo.2"));
    }
}
