//! Operational health checks for galdr.

use anyhow::{Result, bail};

use crate::{catalog, config, ipc, paths, record, setup, style, validate};

/// A green `ok` status line. The 4-char tag + a space keeps every line's text aligned.
fn ok(msg: impl AsRef<str>) {
    println!("{}  {}", style::green("ok"), msg.as_ref());
}

/// An amber `warn` status line.
fn warn(msg: impl AsRef<str>) {
    println!("{} {}", style::amber("warn"), msg.as_ref());
}

/// A red `err` status line.
fn err(msg: impl AsRef<str>) {
    println!("{}  {}", style::red("err"), msg.as_ref());
}

pub fn run() -> Result<()> {
    let mut issues = Vec::new();

    check_path("galdr root", paths::galdr_root().ok(), &mut issues);
    check_path("skills root", paths::skills_root().ok(), &mut issues);

    match config::Config::load() {
        Ok(cfg) => ok(format!("config endpoint is loopback: {}", cfg.endpoint)),
        Err(e) => {
            err(format!("config: {e:#}"));
            issues.push("config is invalid".to_string());
        }
    }

    match ipc::query(&ipc::Request::Ping) {
        Ok(ipc::Response::Pong) => ok("daemon is running"),
        _ => warn("daemon is not running; CLI fallbacks will be used"),
    }

    match record::read_active() {
        Some(active) => ok(format!(
            "active recording: {} ({})",
            active.name, active.rec_id
        )),
        None => ok("no active recording"),
    }

    match catalog::open_in_memory_indexed() {
        Ok(conn) => {
            let recordings = catalog::list_recordings(&conn).unwrap_or_default();
            let skills = catalog::list_skills(&conn).unwrap_or_default();
            let usages = catalog::list_skill_usage(&conn, None).unwrap_or_default();
            let outcomes = catalog::list_skill_outcomes(&conn, None).unwrap_or_default();
            let orphan_count = skills.iter().filter(|skill| skill.orphan).count();
            let draft_count = skills
                .iter()
                .filter(|skill| {
                    matches!(
                        skill.status.as_str(),
                        catalog::STATUS_DRAFT | catalog::STATUS_PARAM_DRAFT
                    )
                })
                .count();
            ok(format!(
                "catalog rebuild check: {} recordings, {} skills, {} usages, {} outcomes",
                recordings.len(),
                skills.len(),
                usages.len(),
                outcomes.len()
            ));
            if orphan_count > 0 {
                warn(format!(
                    "{orphan_count} skill(s) have missing recording provenance"
                ));
            }
            if draft_count > 0 {
                warn(format!("{draft_count} skill(s) are still drafts"));
            }
            report_discoverability(&skills);
            report_validation(&skills);
        }
        Err(e) => {
            err(format!("catalog rebuild check failed: {e:#}"));
            issues.push("catalog cannot be rebuilt from disk".to_string());
        }
    }

    match crate::skill::installed_version() {
        Some(v) if crate::skill::is_current() => {
            ok(format!("galdr skill installed and current (version {v})"))
        }
        Some(v) => warn(format!(
            "galdr skill is stale (installed {v}, binary {}); run `galdr setup skill`",
            env!("CARGO_PKG_VERSION")
        )),
        None => {
            warn("galdr skill not installed; run `galdr setup skill` so your agent can drive galdr")
        }
    }

    match setup::claude_hook_configured() {
        Some(true) => ok("Claude Code PostToolUse hook is configured"),
        Some(false) => {
            warn("Claude Code PostToolUse hook is missing");
            issues.push("Claude Code hook is missing".to_string());
        }
        None => {
            warn("Claude Code settings not found or unreadable");
            issues.push("Claude Code settings are unavailable".to_string());
        }
    }

    if let Ok(root) = paths::frames_root()
        && root.is_dir()
    {
        let leftover = std::fs::read_dir(&root)
            .map(|d| {
                d.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .count()
            })
            .unwrap_or(0);
        if leftover > 0 {
            warn(format!(
                "{leftover} recording(s) have leftover authoring frames (pixels on disk) at {}; they purge on a final distill, or delete the directory",
                root.display()
            ));
        }
    }

    if issues.is_empty() {
        println!("{}", style::green("doctor: ok"));
        Ok(())
    } else {
        bail!("doctor found {} actionable issue(s)", issues.len())
    }
}

/// Reports whether galdr-distilled skills are discoverable by the installed
/// harnesses. A skill the harness can't load is galdr failing at its one job, so
/// this surfaces it (a warning, not an error: `galdr link` fixes it).
fn report_discoverability(skills: &[catalog::SkillRow]) {
    let galdr_skills: Vec<&catalog::SkillRow> = skills
        .iter()
        .filter(|s| s.origin == catalog::ORIGIN_GALDR)
        .collect();
    if galdr_skills.is_empty() {
        return;
    }
    let harnesses: Vec<crate::harness::HarnessInfo> = crate::harness::detect()
        .into_iter()
        .filter(|h| h.detected && crate::harness::skills_dir(&h.key).is_some())
        .collect();
    if harnesses.is_empty() {
        return;
    }
    let mut unreachable = 0;
    for skill in &galdr_skills {
        for h in &harnesses {
            if let Some(dir) = crate::harness::skills_dir(&h.key) {
                let link = dir.join(&skill.skill_name);
                // Same-root (the harness reads the canonical dir) counts as reachable.
                let same_root = crate::paths::skills_root()
                    .map(|r| dir == r)
                    .unwrap_or(false);
                if !same_root && !link.exists() {
                    unreachable += 1;
                    break;
                }
            }
        }
    }
    if unreachable > 0 {
        warn(format!(
            "{unreachable} galdr skill(s) are not discoverable by an installed harness; run `galdr link`"
        ));
    } else {
        ok(format!(
            "{} galdr skill(s) discoverable across {} harness(es)",
            galdr_skills.len(),
            harnesses.len()
        ));
    }
}

/// Runs the content gate over galdr's own installed skills and warns about any that
/// would fail it (e.g. a skill distilled before the gate existed). Scoped to
/// galdr-distilled skills: the shared root also holds hand-authored skills with their
/// own structure, which galdr neither wrote nor judges here (`galdr validate --all`
/// audits those on demand). A warning, not an error: the fix is the operator's, and a
/// pre-existing skill must not break `doctor`.
fn report_validation(skills: &[catalog::SkillRow]) {
    let galdr: Vec<&catalog::SkillRow> = skills
        .iter()
        .filter(|s| s.origin == catalog::ORIGIN_GALDR)
        .collect();
    if galdr.is_empty() {
        return;
    }
    let mut failing = Vec::new();
    for skill in &galdr {
        let Ok(md) = std::fs::read_to_string(&skill.skill_path) else {
            continue;
        };
        let draft = matches!(
            skill.status.as_str(),
            catalog::STATUS_DRAFT | catalog::STATUS_PARAM_DRAFT
        );
        let ctx = validate::ValidationCtx::new(draft, false);
        if validate::validate_skill(&md, &ctx).has_blocking(false) {
            failing.push(skill.skill_name.clone());
        }
    }
    if failing.is_empty() {
        ok(format!(
            "{} galdr skill(s) pass the validation gate",
            galdr.len()
        ));
    } else {
        warn(format!(
            "{} galdr skill(s) would fail the validation gate: {}",
            failing.len(),
            failing.join(", ")
        ));
        println!(
            "     {}",
            style::dim("fix or re-distill them; run `galdr validate <skill>` for the findings")
        );
    }
}

fn check_path(label: &str, path: Option<std::path::PathBuf>, issues: &mut Vec<String>) {
    let Some(path) = path else {
        err(format!("{label}: path unavailable"));
        issues.push(format!("{label} unavailable"));
        return;
    };
    if path.exists() {
        ok(format!("{label}: {}", path.display()));
    } else {
        warn(format!("{label} missing: {}", path.display()));
    }
}
