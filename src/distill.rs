//! Phase 0 distillation: span → `SKILL.md` draft.
//!
//! No LLM here. galdr normalizes the span (one line per step, summarized for
//! reading) and emits a skill draft with an instruction block aimed at the agent
//! itself, which does the fine distillation by reading the span. That way the
//! tracer bullet needs no API key and no cost, and it validates the format.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::span::Event;
use crate::{paths, record, span};

/// Distills recording `id`.
///
/// Without `from`, it emits the skill draft (scaffolding) by reading the span.
/// With `from`, it installs as the final `SKILL.md` the content the agent already
/// distilled into a file in an allowed working area. This second path exists so
/// galdr is the **only** writer of the skills directory: the agent never touches
/// it by hand.
pub fn distill(id: &str, from: Option<&Path>) -> Result<()> {
    let recording = load_recording(id)?;
    let skill_name = format!("galdr-{}", slugify(&recording.name));
    let skill_dir = paths::skill_dir(&skill_name)?;
    std::fs::create_dir_all(&skill_dir)?;
    let skill_path = skill_dir.join("SKILL.md");

    if let Some(src) = from {
        let content = std::fs::read_to_string(src)
            .with_context(|| format!("could not read the distillation at {}", src.display()))?;
        std::fs::write(&skill_path, content)?;
        println!("Skill installed at {}", skill_path.display());
        return Ok(());
    }

    let span_path = paths::span_file(id)?;
    let events = span::read_span(&span_path)
        .with_context(|| format!("could not read span {}", span_path.display()))?;

    let content = render_skill(&skill_name, &recording, &events, &span_path);
    std::fs::write(&skill_path, content)?;

    println!("Skill draft written to {}", skill_path.display());
    println!("Normalized steps: {}", events.len());
    println!();
    println!("Fine distillation (done by the agent):");
    println!("  1. Read the span {}", span_path.display());
    println!("  2. Write the refined skill to a temporary file (working area).");
    println!("  3. Install it:  galdr distill {id} --from <that-file>");
    Ok(())
}

/// Loads the metadata of a closed recording.
fn load_recording(id: &str) -> Result<record::Recording> {
    let rec_path = paths::recording_file(id)?;
    let contents = std::fs::read_to_string(&rec_path)
        .with_context(|| format!("recording {id} not found. Did you run `galdr rec stop`?"))?;
    Ok(serde_json::from_str(&contents)?)
}

/// Turns a name into a slug suitable for a skill directory.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "rec".to_string()
    } else {
        slug
    }
}

/// Truncates text to `max` characters, adding an ellipsis if it is cut.
fn truncate(text: &str, max: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let head: String = one_line.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Summarizes a tool call's input on one line, according to the tool.
fn summarize_input(tool_name: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input.get(key).and_then(|v| v.as_str()).map(str::to_string);

    let raw = match tool_name {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => field("file_path"),
        "Glob" => field("pattern"),
        "Grep" => field("pattern").map(|p| {
            field("path")
                .map(|path| format!("{p}  in {path}"))
                .unwrap_or(p)
        }),
        "WebFetch" | "WebSearch" => field("url").or_else(|| field("query")),
        _ => None,
    };

    let raw = raw.unwrap_or_else(|| match input {
        serde_json::Value::Object(map) => {
            let keys: Vec<&str> = map.keys().map(String::as_str).collect();
            format!("fields: {}", keys.join(", "))
        }
        serde_json::Value::Null => "(no input)".to_string(),
        other => other.to_string(),
    });

    truncate(&raw, 160)
}

/// Composes the content of the `SKILL.md` draft.
fn render_skill(
    skill_name: &str,
    recording: &record::Recording,
    events: &[Event],
    span_path: &Path,
) -> String {
    let mut out = String::new();

    // Skill frontmatter. The description is a draft the agent refines.
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "name: {skill_name}");
    let _ = writeln!(
        out,
        "description: \"[galdr DRAFT] Reproduces the recorded task \\\"{}\\\" ({} steps). The agent must sharpen this description so matching is precise.\"",
        recording.name,
        events.len()
    );
    let _ = writeln!(out, "---");
    let _ = writeln!(out);

    let _ = writeln!(out, "# {skill_name}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "> Draft generated by `galdr distill` from a recording. This is **not** the final skill: it is the scaffolding. The agent completes the marked sections by reading the span."
    );
    let _ = writeln!(out);

    // Recording metadata.
    let _ = writeln!(out, "## Provenance");
    let _ = writeln!(out);
    let _ = writeln!(out, "- rec_id: `{}`", recording.rec_id);
    let _ = writeln!(
        out,
        "- recorded: {} → {}",
        recording.started_at, recording.ended_at
    );
    if let Some(cwd) = &recording.cwd {
        let _ = writeln!(out, "- cwd: `{cwd}`");
    }
    let _ = writeln!(out, "- span (raw): `{}`", span_path.display());
    let _ = writeln!(out);

    // Goal: completed by the agent.
    let _ = writeln!(out, "## Goal");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "<!-- TODO(agent): one or two sentences on WHAT this skill achieves and WHEN to use it. -->"
    );
    let _ = writeln!(out);

    // Normalized steps from the span.
    let _ = writeln!(out, "## Recorded steps (normalized)");
    let _ = writeln!(out);
    if events.is_empty() {
        let _ = writeln!(out, "_(the recording captured no steps)_");
    } else {
        for event in events {
            let summary = summarize_input(&event.tool_name, &event.tool_input);
            let _ = writeln!(
                out,
                "{}. **{}** — {}",
                event.seq + 1,
                event.tool_name,
                summary
            );
        }
    }
    let _ = writeln!(out);

    // Distillation instructions aimed at the agent.
    let _ = writeln!(out, "## Distillation instructions (for the agent)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Read the full span at `{}` (one JSON line per step, with `tool_input` and `tool_response`) and rewrite this file as a reproducible skill:",
        span_path.display()
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "1. **Goal**: infer what the task does end to end and fill the section above."
    );
    let _ = writeln!(
        out,
        "2. **Description** (frontmatter): rewrite it so matching is precise; drop the `[galdr DRAFT]` prefix."
    );
    let _ = writeln!(
        out,
        "3. **Parameters**: identify which values are specific to this recording (paths, names, text) and turn them into parameters with judgment, not literals."
    );
    let _ = writeln!(
        out,
        "4. **Procedure**: turn the steps into actionable instructions; group the incidental, keep the essential order."
    );
    let _ = writeln!(
        out,
        "5. **Success criteria**: add how to verify the task came out right (each step's `tool_response` gives hints)."
    );
    let _ = writeln!(
        out,
        "6. **Robustness**: note preconditions and what to do if a step fails."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Delete this instruction section when you are done.");
    let _ = writeln!(out);

    out
}

#[cfg(test)]
mod tests {
    use super::{slugify, summarize_input, truncate};

    #[test]
    fn slugify_normalizes_names() {
        assert_eq!(slugify("Git Change Summary"), "git-change-summary");
        assert_eq!(slugify("  weird__name!! "), "weird-name");
        assert_eq!(slugify("!!!"), "rec");
    }

    #[test]
    fn truncate_collapses_and_caps() {
        assert_eq!(truncate("a b  c", 80), "a b c");
        assert!(truncate(&"x".repeat(200), 10).ends_with('…'));
    }

    #[test]
    fn summarize_reads_tool_specific_fields() {
        assert_eq!(
            summarize_input("Bash", &serde_json::json!({ "command": "git status" })),
            "git status"
        );
        assert_eq!(
            summarize_input("Write", &serde_json::json!({ "file_path": "/tmp/x.md" })),
            "/tmp/x.md"
        );
        assert_eq!(
            summarize_input("Unknown", &serde_json::json!({ "a": 1, "b": 2 })),
            "fields: a, b"
        );
    }
}
