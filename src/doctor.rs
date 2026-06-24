//! Operational health checks for galdr.

use anyhow::{Result, bail};

use crate::{catalog, config, ipc, paths, record, setup};

pub fn run() -> Result<()> {
    let mut issues = Vec::new();

    check_path("galdr root", paths::galdr_root().ok(), &mut issues);
    check_path("skills root", paths::skills_root().ok(), &mut issues);

    match config::Config::load() {
        Ok(cfg) => println!("ok   config endpoint is loopback: {}", cfg.endpoint),
        Err(err) => {
            println!("err  config: {err:#}");
            issues.push("config is invalid".to_string());
        }
    }

    match ipc::query(&ipc::Request::Ping) {
        Ok(ipc::Response::Pong) => println!("ok   daemon is running"),
        _ => println!("warn daemon is not running; CLI fallbacks will be used"),
    }

    match record::read_active() {
        Some(active) => println!("ok   active recording: {} ({})", active.name, active.rec_id),
        None => println!("ok   no active recording"),
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
            println!(
                "ok   catalog rebuild check: {} recordings, {} skills, {} usages, {} outcomes",
                recordings.len(),
                skills.len(),
                usages.len(),
                outcomes.len()
            );
            if orphan_count > 0 {
                println!("warn {orphan_count} skill(s) have missing recording provenance");
            }
            if draft_count > 0 {
                println!("warn {draft_count} skill(s) are still drafts");
            }
            report_discoverability(&skills);
        }
        Err(err) => {
            println!("err  catalog rebuild check failed: {err:#}");
            issues.push("catalog cannot be rebuilt from disk".to_string());
        }
    }

    match setup::claude_hook_configured() {
        Some(true) => println!("ok   Claude Code PostToolUse hook is configured"),
        Some(false) => {
            println!("warn Claude Code PostToolUse hook is missing");
            issues.push("Claude Code hook is missing".to_string());
        }
        None => {
            println!("warn Claude Code settings not found or unreadable");
            issues.push("Claude Code settings are unavailable".to_string());
        }
    }

    if issues.is_empty() {
        println!("doctor: ok");
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
        println!(
            "warn {unreachable} galdr skill(s) are not discoverable by an installed harness; run `galdr link`"
        );
    } else {
        println!(
            "ok   {} galdr skill(s) discoverable across {} harness(es)",
            galdr_skills.len(),
            harnesses.len()
        );
    }
}

fn check_path(label: &str, path: Option<std::path::PathBuf>, issues: &mut Vec<String>) {
    let Some(path) = path else {
        println!("err  {label}: path unavailable");
        issues.push(format!("{label} unavailable"));
        return;
    };
    if path.exists() {
        println!("ok   {label}: {}", path.display());
    } else {
        println!("warn {label} missing: {}", path.display());
    }
}
