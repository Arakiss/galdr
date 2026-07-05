//! Append-only skill usage and outcome-label capture.
//!
//! These logs are the durable source for later supervised evaluation. The SQLite
//! catalog indexes them for queries, but the JSONL files remain the truth.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{catalog, paths, record};

pub const OUTCOME_UNKNOWN: &str = "unknown";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageEvent {
    pub event_id: String,
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_hash: Option<String>,
    pub rec_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_kind: Option<String>,
    pub outcome: String,
    pub retries: u32,
    pub manual_intervention_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOutcomeEvent {
    pub event_id: String,
    pub skill_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rec_id: Option<String>,
    pub evaluator_kind: String,
    pub label: String,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub evidence_refs: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct UsageInput {
    pub skill_name: String,
    pub rec_id: String,
    pub task_kind: Option<String>,
    pub outcome: String,
    pub retries: u32,
    pub manual_intervention_count: u32,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OutcomeInput {
    pub skill_name: String,
    pub rec_id: Option<String>,
    pub evaluator_kind: String,
    pub label: String,
    pub confidence: f64,
    pub notes: Option<String>,
}

pub fn record_usage(input: UsageInput) -> Result<SkillUsageEvent> {
    if input.skill_name.trim().is_empty() {
        bail!("skill name cannot be empty");
    }
    if input.rec_id.trim().is_empty() {
        bail!("rec_id cannot be empty");
    }
    let event = SkillUsageEvent {
        event_id: Ulid::new().to_string(),
        skill_hash: skill_hash(&input.skill_name).ok(),
        skill_name: input.skill_name,
        rec_id: input.rec_id,
        task_kind: input.task_kind.filter(|value| !value.trim().is_empty()),
        outcome: normalize_label(&input.outcome, OUTCOME_UNKNOWN),
        retries: input.retries,
        manual_intervention_count: input.manual_intervention_count,
        notes: input.notes.filter(|value| !value.trim().is_empty()),
        created_at: record::now_rfc3339(),
    };
    paths::ensure_dirs()?;
    append_jsonl(&paths::skill_usage_log()?, &event)?;
    let _ = catalog::sync_skill_usage(&event);
    Ok(event)
}

pub fn record_outcome(input: OutcomeInput) -> Result<SkillOutcomeEvent> {
    if input.skill_name.trim().is_empty() {
        bail!("skill name cannot be empty");
    }
    if !(0.0..=1.0).contains(&input.confidence) {
        bail!("confidence must be between 0 and 1");
    }
    let event = SkillOutcomeEvent {
        event_id: Ulid::new().to_string(),
        skill_name: input.skill_name,
        rec_id: input.rec_id.filter(|value| !value.trim().is_empty()),
        evaluator_kind: normalize_label(&input.evaluator_kind, "human"),
        label: normalize_label(&input.label, OUTCOME_UNKNOWN),
        confidence: input.confidence,
        notes: input.notes.filter(|value| !value.trim().is_empty()),
        evidence_refs: serde_json::json!({}),
        created_at: record::now_rfc3339(),
    };
    paths::ensure_dirs()?;
    append_jsonl(&paths::skill_outcomes_log()?, &event)?;
    let _ = catalog::sync_skill_outcome(&event);
    Ok(event)
}

/// Whether a skill is currently installed under the skills root. Used to warn —
/// not block — when an outcome is recorded against a name that does not exist,
/// since a typo silently poisons the very training data this lane collects. We
/// still record it: a skill may have been legitimately uninstalled after use.
pub fn skill_exists(skill_name: &str) -> bool {
    paths::skill_dir(skill_name)
        .map(|dir| dir.join("SKILL.md").exists())
        .unwrap_or(false)
}

pub fn read_usage_log(path: &Path) -> Result<Vec<SkillUsageEvent>> {
    read_jsonl(path)
}

pub fn read_outcome_log(path: &Path) -> Result<Vec<SkillOutcomeEvent>> {
    read_jsonl(path)
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

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut events = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<T>(line) {
            events.push(event);
        }
    }
    Ok(events)
}

fn normalize_label(value: &str, fallback: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    }
}

pub fn skill_hash(skill_name: &str) -> Result<String> {
    let path = paths::skill_dir(skill_name)?.join("SKILL.md");
    let bytes =
        std::fs::read(&path).with_context(|| format!("could not read {}", path.display()))?;
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_labels_for_queryable_rows() {
        assert_eq!(normalize_label("Needs Review", "unknown"), "needs_review");
        assert_eq!(normalize_label(" ", "unknown"), "unknown");
    }
}
