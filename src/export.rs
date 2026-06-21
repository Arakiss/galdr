//! Recording export helpers.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::{catalog, paths, record, span};

pub fn export_recording(id: &str, out: &Path, include_raw: bool, redact: bool) -> Result<()> {
    std::fs::create_dir_all(out)
        .with_context(|| format!("could not create export directory {}", out.display()))?;

    let rec_path = paths::recording_file(id)?;
    let recording_json = std::fs::read_to_string(&rec_path)
        .with_context(|| format!("recording {id} not found. Did you run `galdr rec stop`?"))?;
    let recording: record::Recording = serde_json::from_str(&recording_json)?;
    std::fs::write(
        out.join("recording.json"),
        serde_json::to_string_pretty(&recording)?,
    )?;

    let conn = catalog::open_in_memory_indexed()?;
    if let Some(detail) = catalog::show_recording(&conn, id)? {
        let mut steps = String::new();
        steps.push_str("# galdr export\n\n");
        steps.push_str(&format!(
            "- rec_id: `{}`\n- name: `{}`\n- steps: {}\n\n",
            recording.rec_id,
            recording.name,
            detail.steps.len()
        ));
        steps.push_str("## Steps\n\n");
        for step in detail.steps {
            steps.push_str(&format!(
                "{}. **{}** — {}\n",
                step.seq + 1,
                step.tool_name,
                step.summary
            ));
        }
        std::fs::write(out.join("steps.md"), steps)?;
    }

    let skills: Vec<_> = catalog::list_skills(&conn)?
        .into_iter()
        .filter(|skill| skill.rec_id.as_deref() == Some(id))
        .collect();
    std::fs::write(
        out.join("skills.json"),
        serde_json::to_string_pretty(&skills)?,
    )?;
    let usages: Vec<_> = catalog::list_skill_usage(&conn, None)?
        .into_iter()
        .filter(|usage| usage.rec_id == id)
        .collect();
    std::fs::write(
        out.join("usage.json"),
        serde_json::to_string_pretty(&usages)?,
    )?;

    let outcomes: Vec<_> = catalog::list_skill_outcomes(&conn, None)?
        .into_iter()
        .filter(|outcome| outcome.rec_id.as_deref() == Some(id))
        .collect();
    std::fs::write(
        out.join("outcomes.json"),
        serde_json::to_string_pretty(&outcomes)?,
    )?;

    if include_raw || redact {
        if include_raw && !redact {
            eprintln!("warning: exporting raw tool payloads; treat the output as sensitive");
        }
        let span_path = paths::span_file(id)?;
        let events = span::read_span(&span_path)?;
        let file_name = if redact {
            "raw.redacted.jsonl"
        } else {
            "raw.jsonl"
        };
        let mut file = std::fs::File::create(out.join(file_name))?;
        for mut event in events {
            if redact {
                redact_value(&mut event.tool_input);
                redact_value(&mut event.tool_response);
            }
            writeln!(file, "{}", serde_json::to_string(&event)?)?;
        }
    }

    println!("export written to {}", out.display());
    Ok(())
}

fn redact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_sensitive_key(key) {
                    *value = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_value(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_value(value);
            }
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "token",
        "secret",
        "api_key",
        "apikey",
        "authorization",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}
