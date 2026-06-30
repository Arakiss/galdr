//! Recorded-event model and span JSONL I/O.
//!
//! The span is the raw source of truth: append-only, one JSON line per observed
//! event, immutable once written. Nothing is deleted or rewritten; the queryable
//! catalog (SQLite) only indexes this.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// The high-level kind of observed event.
///
/// The default is deliberately `ToolCall`, so every span written before human
/// observation existed still deserializes without a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A normal harness tool call captured by `galdr hook`.
    #[default]
    ToolCall,
    /// A human action captured by an observe sensor.
    Human,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::Human => "human",
        }
    }

    pub fn is_tool_call(&self) -> bool {
        *self == Self::ToolCall
    }
}

/// A typed human-observation payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanEvent {
    pub source: HumanSource,
    /// Canonical action name, e.g. `human.browser.click`.
    pub action: HumanAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<HumanTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<HumanValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_ref: Option<String>,
}

/// A typed wrapper around the canonical human action string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HumanAction(pub String);

impl HumanAction {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HumanAction {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// Where a human-observation event came from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HumanSource {
    Browser {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab_id: Option<String>,
    },
    MacApp {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        window_title: Option<String>,
    },
}

/// The semantic target of a human action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanTarget {
    pub primary: TargetLocator,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternates: Vec<TargetLocator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_summary: Option<String>,
}

/// Locator candidates for replay-authoring. Coordinates are debug metadata only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetLocator {
    Role {
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Label {
        value: String,
    },
    Placeholder {
        value: String,
    },
    TestId {
        value: String,
    },
    Css {
        value: String,
    },
    XPath {
        value: String,
    },
}

/// Value policy for a human input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum HumanValue {
    Omitted {
        reason: String,
    },
    Redacted {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chars: Option<usize>,
    },
    Literal {
        value: String,
    },
}

/// A span event: one tool call or human action observed by a sensor.
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
    /// Event kind. Omitted for historical and normal tool-call events.
    #[serde(default, skip_serializing_if = "EventKind::is_tool_call")]
    pub event_kind: EventKind,
    /// Typed payload for human-observation events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human: Option<HumanEvent>,
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
            event_kind: EventKind::ToolCall,
            human: None,
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

    #[test]
    fn old_tool_call_json_defaults_event_kind() {
        let event: Event = serde_json::from_str(
            r#"{"ts":"2026-06-19T00:00:00Z","seq":0,"tool_name":"Bash","tool_input":{"command":"git status"}}"#,
        )
        .unwrap();
        assert_eq!(event.event_kind, EventKind::ToolCall);
        assert!(event.human.is_none());
    }

    #[test]
    fn human_event_roundtrips() {
        let event = Event {
            ts: "2026-06-19T00:00:00Z".into(),
            seq: 0,
            tool_name: "human.browser.click".into(),
            tool_input: serde_json::Value::Null,
            tool_response: serde_json::Value::Null,
            cwd: None,
            session_id: None,
            event_kind: EventKind::Human,
            human: Some(HumanEvent {
                source: HumanSource::Browser {
                    url: Some("https://example.test".into()),
                    title: Some("Example".into()),
                    tab_id: None,
                },
                action: HumanAction::from("human.browser.click"),
                target: Some(HumanTarget {
                    primary: TargetLocator::Role {
                        role: "button".into(),
                        name: Some("Create issue".into()),
                    },
                    alternates: vec![TargetLocator::Css {
                        value: "button#create".into(),
                    }],
                    role: Some("button".into()),
                    name: Some("Create issue".into()),
                    text: None,
                    label: None,
                    placeholder: None,
                    element_summary: None,
                }),
                value: None,
                verification_hint: Some("Confirm the issue was created.".into()),
                frame_ref: None,
            }),
        };

        let encoded = serde_json::to_string(&event).unwrap();
        assert!(encoded.contains(r#""event_kind":"human""#));
        let decoded: Event = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.event_kind, EventKind::Human);
        assert_eq!(
            decoded.human.unwrap().action.as_str(),
            "human.browser.click"
        );
    }
}
