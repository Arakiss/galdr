//! Append-only per-step judgment capture for on-policy distillation.
//!
//! galdr does not judge recordings itself. It ingests judgments produced by an
//! external strong model, reviewer, or agent and stores them as local ledger data.
//! The span remains immutable; judgments are a parallel JSONL signal keyed by
//! `(rec_id, seq)` and grouped by `task_key` for multi-attempt summaries.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{catalog, ipc, paths, record, span, summary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepVerdict {
    Ok,
    Fork,
}

impl StepVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fork => "fork",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepJudgmentEvent {
    pub event_id: String,
    pub rec_id: String,
    pub seq: u64,
    pub verdict: StepVerdict,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<String>,
    pub task_key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub imported: usize,
    pub recordings: usize,
    pub tasks: Vec<String>,
    pub judgments: Vec<StepJudgmentEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForkExample {
    pub rec_id: String,
    pub rationale: String,
    pub suggested_action: Option<String>,
    pub judge: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForkPoint {
    pub seq: u64,
    pub step: u64,
    pub tool_name: String,
    pub summary: String,
    pub judged: usize,
    pub forks: usize,
    pub ok: usize,
    pub examples: Vec<ForkExample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OnPolicySummary {
    pub task_key: String,
    pub recordings: usize,
    pub judged_steps: usize,
    pub forks: usize,
    pub fork_points: Vec<ForkPoint>,
}

#[derive(Debug, Clone, Deserialize)]
struct ImportEnvelope {
    #[serde(default)]
    task_key: Option<String>,
    #[serde(default, alias = "evaluator")]
    judge: Option<String>,
    recordings: Vec<RecordingJudgments>,
}

#[derive(Debug, Clone, Deserialize)]
struct RecordingJudgments {
    rec_id: String,
    #[serde(default)]
    task_key: Option<String>,
    #[serde(default, alias = "evaluator")]
    judge: Option<String>,
    judgments: Vec<IncomingJudgment>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ImportPayload {
    Envelope(ImportEnvelope),
    Recording(RecordingJudgments),
    Recordings(Vec<RecordingJudgments>),
}

#[derive(Debug, Clone, Deserialize)]
struct IncomingJudgment {
    #[serde(default)]
    seq: Option<u64>,
    #[serde(default)]
    step: Option<u64>,
    verdict: StepVerdict,
    rationale: String,
    #[serde(
        default,
        alias = "suggested-action",
        alias = "suggestedAction",
        alias = "suggestion"
    )]
    suggested_action: Option<String>,
}

struct ForkPointBuilder {
    seq: u64,
    tool_name: String,
    summary: String,
    judged: usize,
    forks: usize,
    ok: usize,
    examples: Vec<ForkExample>,
}

pub fn import_json(raw_json: &str) -> Result<ImportReport> {
    let payload: ImportPayload = serde_json::from_str(raw_json).context(
        "judgment input must be a recording object, an array, or an envelope with `recordings`",
    )?;
    let recordings = match payload {
        ImportPayload::Envelope(envelope) => envelope
            .recordings
            .into_iter()
            .map(|mut recording| {
                if recording.task_key.is_none() {
                    recording.task_key = envelope.task_key.clone();
                }
                if recording.judge.is_none() {
                    recording.judge = envelope.judge.clone();
                }
                recording
            })
            .collect::<Vec<_>>(),
        ImportPayload::Recording(recording) => vec![recording],
        ImportPayload::Recordings(recordings) => recordings,
    };
    if recordings.is_empty() {
        bail!("judgment payload must include at least one recording");
    }

    let mut imported = Vec::new();
    for recording in recordings {
        imported.extend(import_recording(recording)?);
    }

    let mut rec_ids = BTreeSet::new();
    let mut tasks = BTreeSet::new();
    for event in &imported {
        rec_ids.insert(event.rec_id.clone());
        tasks.insert(event.task_key.clone());
    }
    let report = ImportReport {
        imported: imported.len(),
        recordings: rec_ids.len(),
        tasks: tasks.into_iter().collect(),
        judgments: imported,
    };
    if !report.judgments.is_empty() {
        ipc::notify_best_effort(&ipc::Request::StepJudgmentsImported {
            judgments: report.judgments.clone(),
        });
    }
    Ok(report)
}

pub fn summarize(task: Option<&str>, reference: Option<&str>) -> Result<OnPolicySummary> {
    let task_key = if let Some(task) = task.map(str::trim).filter(|value| !value.is_empty()) {
        task.to_string()
    } else if let Some(reference) = reference {
        let rec_id = record::resolve_ref(Some(reference))?;
        task_key_for_rec(&rec_id)?
    } else {
        latest_task_key()?.ok_or_else(|| anyhow::anyhow!("no per-step judgments recorded yet"))?
    };
    Ok(summary_for_task(&task_key))
}

pub fn render_summary(summary: &OnPolicySummary) -> String {
    let mut out = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(
        out,
        "on-policy judgments \"{}\" — {} recording(s), {} judged step(s), {} fork(s)",
        summary.task_key, summary.recordings, summary.judged_steps, summary.forks
    );
    if summary.fork_points.is_empty() {
        let _ = writeln!(out, "(no measured fork points)");
        return out;
    }
    let _ = writeln!(out);
    for point in &summary.fork_points {
        let _ = writeln!(
            out,
            "step {}: {}/{} fork(s) — {} {}",
            point.step, point.forks, point.judged, point.tool_name, point.summary
        );
        for example in point.examples.iter().take(3) {
            let action = example
                .suggested_action
                .as_deref()
                .map(|value| format!(" do: {value}"))
                .unwrap_or_default();
            let judge = example
                .judge
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  - {}{}: {}{}",
                example.rec_id, judge, example.rationale, action
            );
        }
    }
    out
}

pub fn summaries_for_distill(
    recording: &record::Recording,
    steps: &[span::Event],
) -> Vec<ForkPoint> {
    let task_key = task_key_for_rec(&recording.rec_id).unwrap_or_else(|_| recording.name.clone());
    let mut points = summary_for_task(&task_key).fork_points;
    let step_meta = step_metadata(steps);
    for point in &mut points {
        if let Some((tool_name, summary)) = step_meta.get(&point.seq) {
            point.tool_name = tool_name.clone();
            point.summary = summary.clone();
        }
    }
    points
}

pub fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
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

fn import_recording(input: RecordingJudgments) -> Result<Vec<StepJudgmentEvent>> {
    let rec_id = record::resolve_ref(Some(&input.rec_id))?;
    let recording = load_recording(&rec_id)?;
    if input.judgments.is_empty() {
        bail!("recording `{}` has no judgments", input.rec_id);
    }
    let task_key = input
        .task_key
        .map(clean_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| recording.name.clone());
    let judge = input
        .judge
        .map(clean_text)
        .filter(|value| !value.is_empty());

    let mut events = Vec::new();
    for judgment in input.judgments {
        let seq = input_seq(&judgment)?;
        if seq >= recording.steps as u64 {
            bail!(
                "judgment step {} is outside recording `{rec_id}` ({} step(s))",
                seq + 1,
                recording.steps
            );
        }
        let rationale = clean_text(judgment.rationale);
        if rationale.is_empty() {
            bail!("judgment for step {} must include a rationale", seq + 1);
        }
        let suggested_action = judgment
            .suggested_action
            .map(clean_text)
            .filter(|value| !value.is_empty());
        events.push(StepJudgmentEvent {
            event_id: Ulid::new().to_string(),
            rec_id: rec_id.clone(),
            seq,
            verdict: judgment.verdict,
            rationale,
            suggested_action,
            judge: judge.clone(),
            task_key: task_key.clone(),
            created_at: record::now_rfc3339(),
        });
    }

    paths::ensure_dirs()?;
    let path = paths::judgments_file(&rec_id)?;
    for event in &events {
        append_jsonl(&path, event)?;
    }
    let _ = catalog::sync_step_judgments(&events);
    Ok(events)
}

fn summary_for_task(task_key: &str) -> OnPolicySummary {
    let events = all_judgments()
        .into_iter()
        .filter(|event| event.task_key == task_key)
        .collect::<Vec<_>>();
    let mut rec_ids = BTreeSet::new();
    let mut by_seq: BTreeMap<u64, ForkPointBuilder> = BTreeMap::new();
    let mut forks = 0usize;

    for event in &events {
        rec_ids.insert(event.rec_id.clone());
        let step_meta = step_metadata(&read_events(&event.rec_id));
        let (tool_name, step_summary) = step_meta
            .get(&event.seq)
            .cloned()
            .unwrap_or_else(|| ("(missing)".to_string(), "(step not found)".to_string()));
        let entry = by_seq.entry(event.seq).or_insert_with(|| ForkPointBuilder {
            seq: event.seq,
            tool_name,
            summary: step_summary,
            judged: 0,
            forks: 0,
            ok: 0,
            examples: Vec::new(),
        });
        entry.judged += 1;
        match event.verdict {
            StepVerdict::Ok => entry.ok += 1,
            StepVerdict::Fork => {
                entry.forks += 1;
                forks += 1;
                entry.examples.push(ForkExample {
                    rec_id: event.rec_id.clone(),
                    rationale: event.rationale.clone(),
                    suggested_action: event.suggested_action.clone(),
                    judge: event.judge.clone(),
                    created_at: event.created_at.clone(),
                });
            }
        }
    }

    let fork_points = by_seq
        .into_values()
        .filter(|point| point.forks > 0)
        .map(|point| ForkPoint {
            seq: point.seq,
            step: point.seq + 1,
            tool_name: point.tool_name,
            summary: point.summary,
            judged: point.judged,
            forks: point.forks,
            ok: point.ok,
            examples: point.examples,
        })
        .collect();

    OnPolicySummary {
        task_key: task_key.to_string(),
        recordings: rec_ids.len(),
        judged_steps: events.len(),
        forks,
        fork_points,
    }
}

fn task_key_for_rec(rec_id: &str) -> Result<String> {
    let mut latest: Option<StepJudgmentEvent> = None;
    let path = paths::judgments_file(rec_id)?;
    for event in read_jsonl::<StepJudgmentEvent>(&path)? {
        latest = Some(event);
    }
    if let Some(event) = latest {
        return Ok(event.task_key);
    }
    Ok(load_recording(rec_id)?.name)
}

fn latest_task_key() -> Result<Option<String>> {
    Ok(all_judgments()
        .into_iter()
        .max_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.event_id.cmp(&b.event_id))
        })
        .map(|event| event.task_key))
}

fn all_judgments() -> Vec<StepJudgmentEvent> {
    let Ok(dir) = paths::judgments_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(mut events) = read_jsonl::<StepJudgmentEvent>(&path) {
                out.append(&mut events);
            }
        }
    }
    out
}

fn load_recording(rec_id: &str) -> Result<record::Recording> {
    let path = paths::recording_file(rec_id)?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("recording {rec_id} not found. Did you run `galdr rec stop`?"))?;
    Ok(serde_json::from_str(&raw)?)
}

fn read_events(rec_id: &str) -> Vec<span::Event> {
    paths::span_file(rec_id)
        .ok()
        .and_then(|path| span::read_span(&path).ok())
        .unwrap_or_default()
}

fn step_metadata(events: &[span::Event]) -> HashMap<u64, (String, String)> {
    events
        .iter()
        .map(|event| {
            (
                event.seq,
                (event.tool_name.clone(), summary::summarize_event(event)),
            )
        })
        .collect()
}

fn input_seq(judgment: &IncomingJudgment) -> Result<u64> {
    if let Some(seq) = judgment.seq {
        return Ok(seq);
    }
    let Some(step) = judgment.step else {
        bail!("each judgment needs either `seq` (0-based) or `step` (1-based)");
    };
    if step == 0 {
        bail!("`step` is 1-based and must be greater than zero");
    }
    Ok(step - 1)
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

fn clean_text(value: String) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_step_or_seq_inputs() {
        let from_step: IncomingJudgment =
            serde_json::from_str(r#"{"step":2,"verdict":"fork","rationale":"wrong file"}"#)
                .unwrap();
        assert_eq!(input_seq(&from_step).unwrap(), 1);

        let from_seq: IncomingJudgment =
            serde_json::from_str(r#"{"seq":0,"verdict":"ok","rationale":"fine"}"#).unwrap();
        assert_eq!(input_seq(&from_seq).unwrap(), 0);
    }
}
