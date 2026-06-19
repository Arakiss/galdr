//! Parametrization by diffing two recordings of the same task.
//!
//! Two runs of one task differ only in their inputs: a different repo, a different
//! output path, a different count. galdr aligns the two step sequences, then reads
//! the differences off the aligned pairs — fields that stay the same are
//! constants, fields that change are candidate parameters.
//!
//! The aligner is a hand-rolled global alignment (Needleman–Wunsch with
//! match-or-gap and no substitution, which reduces to a longest-common-subsequence
//! over step *shapes*). A shape is the tool plus the sorted top-level input keys
//! (plus the command's first token for `Bash`), so steps line up structurally
//! rather than by literal payload. When the two runs do not align cleanly the
//! report is stamped low-confidence rather than forced into a 1:1 mapping.

use std::fmt::Write as _;

use anyhow::{Context, Result};

use crate::span::Event;
use crate::{paths, record, span};

/// One position in the global alignment of the two step sequences.
#[derive(Debug, Clone)]
pub struct AlignedStep {
    pub a: Option<usize>,
    pub b: Option<usize>,
    /// Both sides present and their shapes equal.
    pub matched: bool,
}

/// A candidate parameter: a leaf field that differs across an aligned pair.
#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub tool_name: String,
    pub json_path: String,
    /// 1-based index of the step in run A (for display).
    pub step: usize,
    pub value_a: String,
    pub value_b: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Low,
}

pub struct DiffReport {
    pub name_a: String,
    pub name_b: String,
    pub events_a: Vec<Event>,
    pub events_b: Vec<Event>,
    pub alignment: Vec<AlignedStep>,
    pub parameters: Vec<Parameter>,
    pub constants: usize,
    pub matched: usize,
    pub confidence: Confidence,
    pub notes: Vec<String>,
}

/// The structural signature of a step: tool + sorted top-level input keys, plus
/// the first command token for `Bash`. Steps with the same shape are alignable.
fn shape_key(event: &Event) -> String {
    let mut keys: Vec<&str> = match &event.tool_input {
        serde_json::Value::Object(map) => map.keys().map(String::as_str).collect(),
        _ => Vec::new(),
    };
    keys.sort_unstable();
    let mut key = format!("{}:{}", event.tool_name, keys.join(","));
    if event.tool_name == "Bash" {
        let first = event
            .tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or("");
        let _ = write!(key, "|{first}");
    }
    key
}

/// Global alignment over the two shape sequences: match (equal shape) or gap, no
/// substitution. With match=1 and gap=0 this is a longest-common-subsequence, so
/// the matched pairs are order-preserving and divergence falls out as gaps.
fn align(shapes_a: &[String], shapes_b: &[String]) -> Vec<AlignedStep> {
    let m = shapes_a.len();
    let n = shapes_b.len();
    // dp[i][j] = best score aligning a[..i] with b[..j].
    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            let diag = if shapes_a[i - 1] == shapes_b[j - 1] {
                dp[i - 1][j - 1] + 1
            } else {
                0
            };
            dp[i][j] = diag.max(dp[i - 1][j]).max(dp[i][j - 1]);
        }
    }

    // Traceback.
    let mut steps = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && shapes_a[i - 1] == shapes_b[j - 1] && dp[i][j] == dp[i - 1][j - 1] + 1
        {
            steps.push(AlignedStep {
                a: Some(i - 1),
                b: Some(j - 1),
                matched: true,
            });
            i -= 1;
            j -= 1;
        } else if j == 0 || (i > 0 && dp[i - 1][j] >= dp[i][j - 1]) {
            steps.push(AlignedStep {
                a: Some(i - 1),
                b: None,
                matched: false,
            });
            i -= 1;
        } else {
            steps.push(AlignedStep {
                a: None,
                b: Some(j - 1),
                matched: false,
            });
            j -= 1;
        }
    }
    steps.reverse();
    steps
}

/// Flattens a JSON value into `(json_path, leaf_string)` pairs.
fn flatten(value: &serde_json::Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(v, &path, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, v) in items.iter().enumerate() {
                flatten(v, &format!("{prefix}[{idx}]"), out);
            }
        }
        serde_json::Value::String(s) => out.push((prefix.to_string(), s.clone())),
        other => out.push((prefix.to_string(), other.to_string())),
    }
}

/// Infers a readable parameter name from the tool, field, and the two values.
fn infer_name(tool: &str, json_path: &str, value_a: &str, value_b: &str) -> String {
    let leaf = json_path.rsplit('.').next().unwrap_or(json_path);
    let leaf = leaf.split('[').next().unwrap_or(leaf);

    if leaf == "command" {
        return "CMD".to_string();
    }
    if value_a.parse::<i64>().is_ok() && value_b.parse::<i64>().is_ok() {
        return "N".to_string();
    }
    if leaf == "file_path" || leaf == "path" || leaf == "notebook_path" {
        return match tool {
            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => "OUT",
            "Read" => "SRC",
            _ => "PATH",
        }
        .to_string();
    }
    if value_a.contains('/') || value_b.contains('/') {
        if leaf.contains("repo") || leaf.contains("dir") || leaf.contains("cwd") {
            return "REPO".to_string();
        }
        return "PATH".to_string();
    }
    String::new()
}

/// Ensures every parameter has a unique name; fills blanks with `P<idx>`.
fn assign_names(params: &mut [Parameter]) {
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (idx, param) in params.iter_mut().enumerate() {
        let base = if param.name.is_empty() {
            format!("P{}", idx + 1)
        } else {
            param.name.clone()
        };
        let entry = counts.entry(base.clone()).or_insert(0);
        *entry += 1;
        param.name = if *entry == 1 {
            base
        } else {
            format!("{base}{}", *entry)
        };
    }
}

/// Analyzes two step sequences into a diff report.
pub fn analyze(name_a: &str, events_a: &[Event], name_b: &str, events_b: &[Event]) -> DiffReport {
    let shapes_a: Vec<String> = events_a.iter().map(shape_key).collect();
    let shapes_b: Vec<String> = events_b.iter().map(shape_key).collect();
    let alignment = align(&shapes_a, &shapes_b);

    let mut parameters = Vec::new();
    let mut constants = 0usize;
    let mut matched = 0usize;

    for step in &alignment {
        let (Some(ia), Some(ib)) = (step.a, step.b) else {
            continue;
        };
        if !step.matched {
            continue;
        }
        matched += 1;
        let (ea, eb) = (&events_a[ia], &events_b[ib]);

        let mut fa = Vec::new();
        let mut fb = Vec::new();
        flatten(&ea.tool_input, "", &mut fa);
        flatten(&eb.tool_input, "", &mut fb);
        let map_b: std::collections::HashMap<&str, &str> =
            fb.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        for (path, va) in &fa {
            let Some(vb) = map_b.get(path.as_str()) else {
                continue;
            };
            if va == vb {
                constants += 1;
            } else {
                let name = infer_name(&ea.tool_name, path, va, vb);
                parameters.push(Parameter {
                    name,
                    tool_name: ea.tool_name.clone(),
                    json_path: path.clone(),
                    step: ia + 1,
                    value_a: va.clone(),
                    value_b: vb.to_string(),
                });
            }
        }
    }
    assign_names(&mut parameters);

    // Confidence and notes.
    let total = shapes_a.len().max(shapes_b.len()).max(1);
    let ratio = matched as f64 / total as f64;
    let gaps_a = alignment.iter().filter(|s| s.b.is_none()).count();
    let gaps_b = alignment.iter().filter(|s| s.a.is_none()).count();

    let mut notes = Vec::new();
    if gaps_a > 0 {
        notes.push(format!(
            "{gaps_a} step(s) in \"{name_a}\" had no counterpart in \"{name_b}\""
        ));
    }
    if gaps_b > 0 {
        notes.push(format!(
            "{gaps_b} step(s) in \"{name_b}\" had no counterpart in \"{name_a}\""
        ));
    }
    if events_a.len() != events_b.len() {
        notes.push(format!(
            "step counts differ: {} vs {}",
            events_a.len(),
            events_b.len()
        ));
    }

    let confidence = if ratio >= 0.7 && gaps_a == 0 && gaps_b == 0 {
        Confidence::High
    } else {
        Confidence::Low
    };

    DiffReport {
        name_a: name_a.to_string(),
        name_b: name_b.to_string(),
        events_a: events_a.to_vec(),
        events_b: events_b.to_vec(),
        alignment,
        parameters,
        constants,
        matched,
        confidence,
        notes,
    }
}

/// Loads both recordings from disk and diffs them.
pub fn compute(id_a: &str, id_b: &str) -> Result<DiffReport> {
    let (name_a, events_a) = load(id_a)?;
    let (name_b, events_b) = load(id_b)?;
    Ok(analyze(&name_a, &events_a, &name_b, &events_b))
}

fn load(id: &str) -> Result<(String, Vec<Event>)> {
    let rec_path = paths::recording_file(id)?;
    let contents = std::fs::read_to_string(&rec_path)
        .with_context(|| format!("recording {id} not found. Did you run `galdr rec stop`?"))?;
    let recording: record::Recording = serde_json::from_str(&contents)?;
    let span_path = paths::span_file(id)?;
    let events = span::read_span(&span_path).unwrap_or_default();
    Ok((recording.name, events))
}

/// Prints the human-readable diff report to stdout.
pub fn render_report(report: &DiffReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "diff  \"{}\" ({} steps)  ↔  \"{}\" ({} steps)",
        report.name_a,
        report.events_a.len(),
        report.name_b,
        report.events_b.len()
    );
    let conf = match report.confidence {
        Confidence::High => "HIGH",
        Confidence::Low => "LOW",
    };
    let total = report.events_a.len().max(report.events_b.len());
    let _ = writeln!(
        out,
        "confidence: {conf}   ({}/{total} steps matched)",
        report.matched
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "Alignment:");
    for step in &report.alignment {
        let left = step
            .a
            .map(|i| step_label(&report.events_a[i], i))
            .unwrap_or_else(|| "      (gap)".to_string());
        let right = step
            .b
            .map(|i| step_label(&report.events_b[i], i))
            .unwrap_or_else(|| "(gap)".to_string());
        let mark = if step.matched { "✓" } else { " " };
        let tag = if step.matched
            && step
                .a
                .is_some_and(|ia| report.parameters.iter().any(|p| p.step == ia + 1))
        {
            "   [param]"
        } else {
            ""
        };
        let _ = writeln!(out, "  {mark} {left}  ↔  {right}{tag}");
    }
    let _ = writeln!(out);

    if report.parameters.is_empty() {
        let _ = writeln!(
            out,
            "Parameters: none (the two runs are identical where they align)"
        );
    } else {
        let _ = writeln!(out, "Parameters:");
        for param in &report.parameters {
            let _ = writeln!(
                out,
                "  {:<6} {} {}   a={}   b={}",
                param.name, param.tool_name, param.json_path, param.value_a, param.value_b
            );
        }
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Constants: {} field(s) identical across matched steps",
        report.constants
    );

    if report.confidence == Confidence::Low && !report.notes.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Alignment notes (low confidence — review before trusting):"
        );
        for note in &report.notes {
            let _ = writeln!(out, "  - {note}");
        }
    }
    out
}

fn step_label(event: &Event, index: usize) -> String {
    let summary = crate::summary::summarize_input(&event.tool_name, &event.tool_input);
    let short: String = summary.chars().take(28).collect();
    format!("{:>2} {:<8} {}", index + 1, event.tool_name, short)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: u64, tool: &str, input: serde_json::Value) -> Event {
        Event {
            ts: "2026-06-19T00:00:00Z".into(),
            seq,
            tool_name: tool.into(),
            tool_input: input,
            tool_response: serde_json::json!({}),
            cwd: None,
            session_id: None,
        }
    }

    #[test]
    fn aligns_identical_shapes_high_confidence() {
        let a = vec![
            ev(0, "Bash", serde_json::json!({ "command": "git status" })),
            ev(1, "Write", serde_json::json!({ "file_path": "/a/out.md" })),
        ];
        let b = vec![
            ev(0, "Bash", serde_json::json!({ "command": "git status" })),
            ev(1, "Write", serde_json::json!({ "file_path": "/b/out.md" })),
        ];
        let report = analyze("a", &a, "b", &b);
        assert_eq!(report.matched, 2);
        assert_eq!(report.confidence, Confidence::High);
        // The diverging file_path is a parameter named OUT; the command is constant.
        assert_eq!(report.parameters.len(), 1);
        let p = &report.parameters[0];
        assert_eq!(p.name, "OUT");
        assert_eq!(p.value_a, "/a/out.md");
        assert_eq!(p.value_b, "/b/out.md");
    }

    #[test]
    fn infers_numeric_parameter() {
        let a = vec![ev(0, "Bash", serde_json::json!({ "command": "seq 5" }))];
        let b = vec![ev(0, "Bash", serde_json::json!({ "command": "seq 10" }))];
        let report = analyze("a", &a, "b", &b);
        // Bash shapes match on the first token "seq"; the full command differs → CMD.
        assert_eq!(report.parameters.len(), 1);
        assert_eq!(report.parameters[0].name, "CMD");
    }

    #[test]
    fn divergent_sequences_are_low_confidence() {
        let a = vec![
            ev(0, "Bash", serde_json::json!({ "command": "git status" })),
            ev(1, "Read", serde_json::json!({ "file_path": "/a.rs" })),
            ev(2, "Write", serde_json::json!({ "file_path": "/a/out.md" })),
        ];
        let b = vec![ev(0, "Glob", serde_json::json!({ "pattern": "*.rs" }))];
        let report = analyze("a", &a, "b", &b);
        assert_eq!(report.confidence, Confidence::Low);
        assert!(!report.notes.is_empty());
    }

    #[test]
    fn unique_parameter_names() {
        let a = vec![ev(
            0,
            "Custom",
            serde_json::json!({ "x": "one", "y": "two" }),
        )];
        let b = vec![ev(
            0,
            "Custom",
            serde_json::json!({ "x": "ONE", "y": "TWO" }),
        )];
        let report = analyze("a", &a, "b", &b);
        assert_eq!(report.parameters.len(), 2);
        // Neither field maps to a known name, so they fall back to P1/P2.
        let names: Vec<&str> = report.parameters.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"P1"));
        assert!(names.contains(&"P2"));
    }
}
