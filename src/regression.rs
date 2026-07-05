//! Skill regression base-case ledger.
//!
//! This is intentionally a guard, not a fake replay engine. A base case records that
//! a skill version should continue to handle a recording. `check` compares the
//! current skill file hash with the pinned hash and reports when the cases need a
//! real replay/review.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{catalog, outcome, paths, record};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionBaseCaseEvent {
    pub event_id: String,
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_hash: Option<String>,
    pub rec_id: String,
    pub case_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct BaseCaseInput {
    pub skill_name: String,
    pub rec_id: String,
    pub case_name: Option<String>,
    pub reference_version: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegressionCaseCheck {
    pub event_id: String,
    pub case_name: String,
    pub rec_id: String,
    pub pinned_skill_hash: Option<String>,
    pub current_skill_hash: Option<String>,
    pub reference_version: Option<String>,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegressionCheck {
    pub skill_name: String,
    pub current_skill_hash: Option<String>,
    pub base_cases: Vec<RegressionCaseCheck>,
    pub changed_unverified: usize,
    pub missing_skill: bool,
}

pub fn record_base_case(input: BaseCaseInput) -> Result<RegressionBaseCaseEvent> {
    let skill_name = input.skill_name.trim();
    if skill_name.is_empty() {
        bail!("skill name cannot be empty");
    }
    if input.rec_id.trim().is_empty() {
        bail!("rec_id cannot be empty");
    }
    ensure_recording_exists(&input.rec_id)?;
    let skill_hash = outcome::skill_hash(skill_name)
        .with_context(|| format!("could not hash installed skill `{skill_name}`"))?;
    let case_name = input
        .case_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| input.rec_id.clone());
    let event = RegressionBaseCaseEvent {
        event_id: Ulid::new().to_string(),
        skill_name: skill_name.to_string(),
        skill_hash: Some(skill_hash),
        rec_id: input.rec_id,
        case_name,
        reference_version: clean_opt(input.reference_version),
        notes: clean_opt(input.notes),
        created_at: record::now_rfc3339(),
    };
    paths::ensure_dirs()?;
    append_jsonl(&paths::regression_base_cases_log()?, &event)?;
    let _ = catalog::sync_regression_base_case(&event);
    Ok(event)
}

pub fn status(skill_name: &str) -> RegressionCheck {
    check_skill(skill_name).unwrap_or_else(|_| RegressionCheck {
        skill_name: skill_name.to_string(),
        current_skill_hash: outcome::skill_hash(skill_name).ok(),
        base_cases: Vec::new(),
        changed_unverified: 0,
        missing_skill: !outcome::skill_exists(skill_name),
    })
}

pub fn render_status(status: &RegressionCheck) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "regression guard — Regression base cases — {}",
        status.skill_name
    );
    if status.base_cases.is_empty() {
        let _ = writeln!(
            out,
            "(no base cases pinned yet — use `galdr regress pin --skill <name> --rec <id> --case <label>`)"
        );
        return out;
    }
    if status.missing_skill {
        let _ = writeln!(out, "skill file is missing; all base cases need review");
    } else if status.changed_unverified > 0 {
        let _ = writeln!(
            out,
            "{} changed/unverified base case(s): skill hash changed since pin and needs real replay/review",
            status.changed_unverified
        );
    } else {
        let _ = writeln!(
            out,
            "all pinned base cases still match the current skill hash"
        );
    }
    let _ = writeln!(
        out,
        "galdr only checks the ledger/hash guard here; it does not execute natural-language skills."
    );
    let _ = writeln!(out);
    for case in &status.base_cases {
        let _ = writeln!(
            out,
            "- {} ({}) rec {} pinned {}",
            case.case_name,
            case.status,
            case.rec_id,
            case.pinned_skill_hash.as_deref().unwrap_or("unknown")
        );
    }
    out
}

pub fn read_base_cases_log(path: &Path) -> Result<Vec<RegressionBaseCaseEvent>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut events = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<RegressionBaseCaseEvent>(line) {
            events.push(event);
        }
    }
    Ok(events)
}

pub fn check_skill(skill_name: &str) -> Result<RegressionCheck> {
    let current = outcome::skill_hash(skill_name).ok();
    let cases = read_base_cases_log(&paths::regression_base_cases_log()?)?
        .into_iter()
        .filter(|event| event.skill_name == skill_name)
        .map(|event| {
            let status = match (current.as_deref(), event.skill_hash.as_deref()) {
                (None, _) => "missing_skill",
                (Some(_), None) => "unpinned",
                (Some(now), Some(pinned)) if now == pinned => "unchanged",
                (Some(_), Some(_)) => "changed_unverified",
            }
            .to_string();
            RegressionCaseCheck {
                event_id: event.event_id,
                case_name: event.case_name,
                rec_id: event.rec_id,
                pinned_skill_hash: event.skill_hash,
                current_skill_hash: current.clone(),
                reference_version: event.reference_version,
                status,
                notes: event.notes,
                created_at: event.created_at,
            }
        })
        .collect::<Vec<_>>();
    let changed_unverified = cases
        .iter()
        .filter(|case| case.status == "changed_unverified")
        .count();
    Ok(RegressionCheck {
        skill_name: skill_name.to_string(),
        current_skill_hash: current.clone(),
        base_cases: cases,
        changed_unverified,
        missing_skill: current.is_none(),
    })
}

fn ensure_recording_exists(rec_id: &str) -> Result<()> {
    let path = paths::recording_file(rec_id)?;
    if !path.exists() {
        bail!("recording {rec_id} not found. Did you run `galdr rec stop`?");
    }
    Ok(())
}

fn append_jsonl<T: Serialize>(path: &Path, event: &T) -> Result<()> {
    let line = serde_json::to_string(event)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn clean_opt(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
