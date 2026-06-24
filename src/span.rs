//! Recorded-event model and span JSONL I/O.
//!
//! The span is the raw source of truth: append-only, one JSON line per tool call,
//! immutable once written. Nothing is deleted or rewritten; the queryable catalog
//! (SQLite) arrives in a later phase and only indexes this.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A span event: one tool call observed by the sensor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// RFC3339 (UTC) timestamp of when it was recorded.
    pub ts: String,
    /// Index of the event within the recording, starting at 0.
    pub seq: u64,
    /// Tool name (`Bash`, `Write`, `Edit`, ...).
    pub tool_name: String,
    /// Tool input, exactly as the harness emits it.
    #[serde(default)]
    pub tool_input: serde_json::Value,
    /// Tool response, exactly as the harness emits it.
    #[serde(default)]
    pub tool_response: serde_json::Value,
    /// Working directory at the time of the call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The agent session identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Appends an event to the span. Creates the file if missing; never truncates.
pub fn append_event(span_path: &Path, event: &Event) -> Result<()> {
    let line = serde_json::to_string(event)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(span_path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Flushes the span to stable storage. Called once when a recording is closed, so
/// the whole recording is durable before its metadata is written — without paying
/// an `fsync` on the sensor's hot path, which the design keeps instantaneous. Any
/// failure (e.g. the file is gone) is the caller's to treat as best-effort.
pub fn fsync(span_path: &Path) -> Result<()> {
    OpenOptions::new().read(true).open(span_path)?.sync_all()?;
    Ok(())
}

/// Counts the non-empty lines of the span. Returns 0 if the file does not exist,
/// so the sensor gets the next event's `seq` without failing on the first call.
pub fn count_events(span_path: &Path) -> u64 {
    match std::fs::read_to_string(span_path) {
        Ok(contents) => contents.lines().filter(|l| !l.trim().is_empty()).count() as u64,
        Err(_) => 0,
    }
}

/// Reads and parses the whole span. Tolerant: a corrupt line is skipped rather
/// than aborting the read, because the raw must stay inspectable even if a write
/// was left half-finished.
pub fn read_span(span_path: &Path) -> Result<Vec<Event>> {
    let contents = std::fs::read_to_string(span_path)?;
    let mut events = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Event>(line) {
            events.push(event);
        }
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: u64) -> Event {
        Event {
            ts: "2026-06-19T00:00:00Z".into(),
            seq,
            tool_name: "Bash".into(),
            tool_input: serde_json::json!({ "command": "echo hi" }),
            tool_response: serde_json::json!({ "exit_code": 0 }),
            cwd: Some("/tmp".into()),
            session_id: Some("s1".into()),
        }
    }

    #[test]
    fn append_then_read_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("span.jsonl");

        assert_eq!(count_events(&path), 0, "missing file counts as zero events");
        append_event(&path, &sample(0)).unwrap();
        append_event(&path, &sample(1)).unwrap();
        assert_eq!(count_events(&path), 2);

        let events = read_span(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 0);
        assert_eq!(events[1].seq, 1);
        assert_eq!(events[0].tool_name, "Bash");
    }

    #[test]
    fn read_skips_corrupt_lines() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("span.jsonl");

        append_event(&path, &sample(0)).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{{ not valid json").unwrap();

        // count_events counts raw non-empty lines; read_span skips the corrupt one.
        assert_eq!(count_events(&path), 2);
        assert_eq!(read_span(&path).unwrap().len(), 1);
    }
}
