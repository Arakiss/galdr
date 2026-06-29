//! Replay reliability: how often a distilled skill reproduces well, measured from the
//! real outcome ledger.
//!
//! galdr cannot execute a natural-language skill — interpreting and running it is the
//! agent's job — so it cannot synthesize a replay or score one on its own. What it can
//! do is aggregate the outcomes you record with `galdr outcome usage` after each real
//! replay into a **per-skill hit-rate** and an **effort cost** (retries, manual
//! interventions). That is the evidence a capability-and-shape test cannot give: not
//! "the skill has the right structure" but "it reproduced cleanly N out of M times in
//! production." With no recorded outcomes there is nothing to measure, and the report
//! says so rather than inventing a number.

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;

use crate::catalog;

/// The minimal usage signal the aggregator needs (one recorded replay outcome).
struct UsageLite {
    skill: String,
    outcome: String,
    retries: i64,
    interventions: i64,
}

/// Reliability of one skill, aggregated over its recorded replays.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillReliability {
    pub skill_name: String,
    pub uses: usize,
    pub success: usize,
    pub partial: usize,
    pub failed: usize,
    /// `success / uses` — the clean-replay hit-rate.
    pub success_rate: f64,
    /// `(success + 0.5 * partial) / uses` — partial replays count half.
    pub effective_rate: f64,
    /// Average retries per replay; a skill that reproduces cleanly needs none.
    pub avg_retries: f64,
    /// Average manual interventions per replay; lower means more autonomous.
    pub avg_interventions: f64,
    /// The skill's readiness score (shape/lint), if the catalog knows it.
    pub readiness: Option<i64>,
    /// `galdr` (distilled) or `external`.
    pub origin: String,
}

/// The whole-fleet view plus the per-skill breakdown.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReliabilityReport {
    /// Total recorded replay outcomes across all skills.
    pub total_replays: usize,
    /// Skills that have at least one recorded outcome.
    pub skills_measured: usize,
    /// Fleet-wide clean-replay hit-rate.
    pub overall_success_rate: f64,
    pub skills: Vec<SkillReliability>,
}

/// Aggregates raw usage rows into a reliability report. Pure (no IO) so it is unit
/// tested directly; `run` supplies the rows and readiness from the catalog.
fn aggregate(
    usages: Vec<UsageLite>,
    meta: &HashMap<String, (Option<i64>, String)>,
) -> ReliabilityReport {
    let mut by_skill: HashMap<String, Vec<UsageLite>> = HashMap::new();
    for usage in usages {
        by_skill.entry(usage.skill.clone()).or_default().push(usage);
    }

    let mut skills: Vec<SkillReliability> = Vec::new();
    let (mut total_replays, mut total_success) = (0usize, 0usize);
    for (skill_name, rows) in by_skill {
        let uses = rows.len();
        let mut success = 0;
        let mut partial = 0;
        let mut failed = 0;
        let mut retries = 0i64;
        let mut interventions = 0i64;
        for row in &rows {
            match row.outcome.to_ascii_lowercase().as_str() {
                "success" => success += 1,
                "partial" => partial += 1,
                "failed" | "failure" => failed += 1,
                _ => {} // an unrecognized label still counts toward `uses`
            }
            retries += row.retries;
            interventions += row.interventions;
        }
        let uses_f = uses as f64;
        let (readiness, origin) = meta
            .get(&skill_name)
            .cloned()
            .unwrap_or((None, catalog::ORIGIN_EXTERNAL.to_string()));
        total_replays += uses;
        total_success += success;
        skills.push(SkillReliability {
            skill_name,
            uses,
            success,
            partial,
            failed,
            success_rate: success as f64 / uses_f,
            effective_rate: (success as f64 + 0.5 * partial as f64) / uses_f,
            avg_retries: retries as f64 / uses_f,
            avg_interventions: interventions as f64 / uses_f,
            readiness,
            origin,
        });
    }

    // Most-exercised first, then the cleaner hit-rate, then name for stability.
    skills.sort_by(|a, b| {
        b.uses
            .cmp(&a.uses)
            .then(b.success_rate.total_cmp(&a.success_rate))
            .then(a.skill_name.cmp(&b.skill_name))
    });

    ReliabilityReport {
        total_replays,
        skills_measured: skills.len(),
        overall_success_rate: if total_replays == 0 {
            0.0
        } else {
            total_success as f64 / total_replays as f64
        },
        skills,
    }
}

/// Runs the benchmark and prints the report (human table or `--json`).
pub fn run(skill: Option<&str>, json: bool) -> Result<()> {
    let conn = catalog::open_in_memory_indexed()?;
    let usages: Vec<UsageLite> = catalog::list_skill_usage(&conn, skill)?
        .into_iter()
        .map(|u| UsageLite {
            skill: u.skill_name,
            outcome: u.outcome,
            retries: u.retries,
            interventions: u.manual_intervention_count,
        })
        .collect();
    let meta: HashMap<String, (Option<i64>, String)> = catalog::list_skills(&conn)?
        .into_iter()
        .map(|s| (s.skill_name, (Some(s.readiness_score), s.origin)))
        .collect();

    let report = aggregate(usages, &meta);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    if report.total_replays == 0 {
        println!("No replay outcomes recorded yet — nothing to measure.");
        println!(
            "After you reuse a distilled skill, record how it went:\n  galdr outcome usage --skill <name> --rec <rec_id> --outcome success|partial|failed\nThis report then turns those into a per-skill replay hit-rate."
        );
        return Ok(());
    }

    println!(
        "Replay reliability — {} recorded outcome(s) across {} skill(s); fleet hit-rate {:.0}%\n",
        report.total_replays,
        report.skills_measured,
        report.overall_success_rate * 100.0,
    );
    println!(
        "{:<32} {:>5} {:>7} {:>6} {:>8} {:>7}",
        "skill", "uses", "hit", "eff", "retries", "ready"
    );
    for s in &report.skills {
        let ready = s
            .readiness
            .map(|r| r.to_string())
            .unwrap_or_else(|| "—".to_string());
        println!(
            "{:<32} {:>5} {:>6.0}% {:>5.0}% {:>8.2} {:>7}",
            truncate(&s.skill_name, 32),
            s.uses,
            s.success_rate * 100.0,
            s.effective_rate * 100.0,
            s.avg_retries,
            ready,
        );
    }
    println!(
        "\nhit = clean replays / uses · eff = partial counts half · retries = avg per replay (lower is better)"
    );
    Ok(())
}

/// Truncates a skill name to fit the column, with an ellipsis if cut.
fn truncate(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        let head: String = name.chars().take(max - 1).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(skill: &str, outcome: &str, retries: i64, interventions: i64) -> UsageLite {
        UsageLite {
            skill: skill.into(),
            outcome: outcome.into(),
            retries,
            interventions,
        }
    }

    fn meta(pairs: &[(&str, i64)]) -> HashMap<String, (Option<i64>, String)> {
        pairs
            .iter()
            .map(|(n, r)| (n.to_string(), (Some(*r), catalog::ORIGIN_GALDR.to_string())))
            .collect()
    }

    #[test]
    fn aggregates_hit_rate_and_effort_per_skill() {
        let usages = vec![
            usage("deploy", "success", 0, 0),
            usage("deploy", "success", 1, 0),
            usage("deploy", "partial", 2, 1),
            usage("deploy", "failed", 3, 2),
        ];
        let report = aggregate(usages, &meta(&[("deploy", 90)]));
        assert_eq!(report.total_replays, 4);
        let s = &report.skills[0];
        assert_eq!((s.uses, s.success, s.partial, s.failed), (4, 2, 1, 1));
        assert_eq!(s.success_rate, 0.5); // 2/4
        assert_eq!(s.effective_rate, 0.625); // (2 + 0.5)/4
        assert_eq!(s.avg_retries, 1.5); // (0+1+2+3)/4
        assert_eq!(s.avg_interventions, 0.75); // (0+0+1+2)/4
        assert_eq!(s.readiness, Some(90));
        assert_eq!(s.origin, catalog::ORIGIN_GALDR);
    }

    #[test]
    fn fleet_rate_spans_all_skills() {
        let usages = vec![
            usage("a", "success", 0, 0),
            usage("a", "failed", 0, 0),
            usage("b", "success", 0, 0),
            usage("b", "success", 0, 0),
        ];
        let report = aggregate(usages, &meta(&[("a", 80), ("b", 100)]));
        // 3 clean replays out of 4 total.
        assert_eq!(report.total_replays, 4);
        assert_eq!(report.skills_measured, 2);
        assert_eq!(report.overall_success_rate, 0.75);
    }

    #[test]
    fn most_used_skill_ranks_first() {
        let usages = vec![
            usage("rare", "success", 0, 0),
            usage("common", "failed", 0, 0),
            usage("common", "success", 0, 0),
            usage("common", "success", 0, 0),
        ];
        let report = aggregate(usages, &meta(&[]));
        assert_eq!(report.skills[0].skill_name, "common");
        assert_eq!(report.skills[0].uses, 3);
        // No metadata → readiness unknown, origin defaults to external.
        assert_eq!(report.skills[0].readiness, None);
        assert_eq!(report.skills[0].origin, catalog::ORIGIN_EXTERNAL);
    }

    #[test]
    fn no_usages_is_an_empty_report_not_a_divide_by_zero() {
        let report = aggregate(Vec::new(), &HashMap::new());
        assert_eq!(report.total_replays, 0);
        assert_eq!(report.overall_success_rate, 0.0);
        assert!(report.skills.is_empty());
    }

    #[test]
    fn unknown_outcome_labels_count_as_uses_but_not_hits() {
        let usages = vec![usage("x", "success", 0, 0), usage("x", "weird", 0, 0)];
        let report = aggregate(usages, &meta(&[("x", 70)]));
        let s = &report.skills[0];
        assert_eq!(s.uses, 2);
        assert_eq!(s.success, 1);
        assert_eq!(s.success_rate, 0.5);
    }
}
