//! Sensor: invoked by the harness after each tool call (PostToolUse).
//!
//! Contract: read the event from stdin and, if a recording is active, append it
//! to the span. Whatever happens, it must **never** break the agent session. The
//! robustness guard (catch panics, exit 0) lives in `main`; here the logic returns
//! `Result` and any error is discarded above.

use std::io::Read;

use anyhow::Result;
use serde::Deserialize;

use crate::ext::{PermissionGate, ProvenanceSink};
use crate::{config, ext, ipc, paths, record, span};

/// Input the harness passes on stdin in PostToolUse. Unknown fields are ignored;
/// missing ones fall back to their default value.
#[derive(Debug, Deserialize)]
struct HookInput {
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: serde_json::Value,
    #[serde(default)]
    tool_response: serde_json::Value,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    transcript_path: Option<String>,
}

/// Processes a PostToolUse event. Returns `Err` on any internal failure; `main`
/// discards it to guarantee exit 0.
pub fn run() -> Result<()> {
    // Robustness test hook: force a failure inside the sensor to verify the agent
    // session survives anyway (see `main`).
    if std::env::var("GALDR_HOOK_FAIL").is_ok() {
        panic!("forced failure for the sensor robustness test");
    }

    // No active recording: nothing to do, fast exit.
    let active = match record::read_active() {
        Some(active) => active,
        None => return Ok(()),
    };

    // Cap how much we read: a hostile or buggy harness could pipe an enormous
    // payload, and an unbounded `read_to_string` would OOM-abort the process before
    // `main`'s panic guard can run — breaking the "never break the session" contract.
    // A real PostToolUse event is kilobytes; 16 MiB is a generous ceiling.
    const MAX_HOOK_BYTES: u64 = 16 * 1024 * 1024;
    let mut buf = String::new();
    std::io::stdin()
        .take(MAX_HOOK_BYTES)
        .read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        return Ok(());
    }
    // If the read hit the cap, the JSON is almost certainly truncated; parsing fails
    // and the event is dropped (the sensor still exits 0), which is the safe outcome.
    let input: HookInput = serde_json::from_str(&buf)?;

    // Session scoping: a single global `active` flag means every concurrent agent
    // session's hook sees this recording. Without scoping, a session in another
    // project would leak its tool calls (and their payloads) into this span. Bind
    // the recording to the first session whose event lands under `origin_cwd`, then
    // record only that session.
    let decision = capture_decision(&active, input.session_id.as_deref(), input.cwd.as_deref());
    if matches!(decision, Capture::Skip) {
        return Ok(());
    }

    let span_path = paths::span_file(&active.rec_id)?;
    let mut event = span::Event {
        ts: record::now_rfc3339(),
        seq: span::count_events(&span_path),
        tool_name: input.tool_name,
        tool_input: input.tool_input,
        tool_response: input.tool_response,
        cwd: input.cwd,
        session_id: input.session_id,
    };

    let capture = config::Config::load_capture();
    if denied_by_capture_policy(&event, &capture) {
        return Ok(());
    }
    // Drop screenshot/base64 blobs before anything else so the span stores the
    // action, never the pixels (smaller, and no on-screen content leaks).
    if capture.strip_screenshots {
        strip_screenshots(&mut event.tool_input);
        strip_screenshots(&mut event.tool_response);
    }
    apply_response_cap(&mut event, capture.max_response_chars);

    // Permission seam: the core allows everything; an external layer may veto.
    let gate = ext::NoopExt;
    if !gate.allow(&event) {
        return Ok(());
    }

    // The append is the truth, first and unconditional.
    span::append_event(&span_path, &event)?;

    // Best-effort, and only after the truth is durable: hint the daemon to index
    // this event. The sensor never waits on or depends on the daemon — a dropped
    // notification is reconciled later from the span on disk.
    ipc::notify_best_effort(&ipc::Request::EventAppended {
        rec_id: active.rec_id.clone(),
        event: event.clone(),
    });

    // Provenance seam: the core records nothing.
    ext::NoopExt.record(&event);

    // Persist any new session binding and capture the transcript once, in a single
    // write of the active flag.
    let new_binding = match decision {
        Capture::RecordAndBind(session_id) => Some(session_id),
        _ => None,
    };
    let new_transcript = input
        .transcript_path
        .filter(|_| active.transcript_path.is_none());
    if new_binding.is_some() || new_transcript.is_some() {
        let updated = record::ActiveRec {
            bound_session: new_binding.or_else(|| active.bound_session.clone()),
            transcript_path: new_transcript.or_else(|| active.transcript_path.clone()),
            ..active
        };
        let _ = record::write_active(&updated);
    }
    Ok(())
}

/// What the sensor should do with one event, given the recording's binding state.
enum Capture {
    /// Drop it: a different session, or a foreign session before binding.
    Skip,
    /// Append it; the recording is already bound (or cannot be session-scoped).
    Record,
    /// Append it and lock the recording onto this session id.
    RecordAndBind(String),
}

/// Decides capture for one event. The rule: a session-less event is always
/// recorded (so harnesses that omit the id, and the no-scope case, keep working);
/// once bound, only the bound session records; before binding, the first event
/// carrying a session id binds — but only if it happened under `origin_cwd`, so a
/// concurrent session in another directory can never claim the recording.
fn capture_decision(
    active: &record::ActiveRec,
    session_id: Option<&str>,
    cwd: Option<&str>,
) -> Capture {
    match active.bound_session.as_deref() {
        Some(bound) => match session_id {
            Some(sid) if sid == bound => Capture::Record,
            Some(_) => Capture::Skip,
            None => Capture::Record,
        },
        None => match session_id {
            None => Capture::Record,
            Some(sid) => {
                let cwd_ok = match (active.origin_cwd.as_deref(), cwd) {
                    (None, _) | (Some(_), None) => true,
                    (Some(origin), Some(cwd)) => path_within(cwd, origin),
                };
                if cwd_ok {
                    Capture::RecordAndBind(sid.to_string())
                } else {
                    Capture::Skip
                }
            }
        },
    }
}

/// True if `path` is `base` itself or lives under it, comparing path components so
/// `/a/bc` is not treated as under `/a/b`.
fn path_within(path: &str, base: &str) -> bool {
    let base = base.trim_end_matches('/');
    path == base || path.starts_with(&format!("{base}/"))
}

fn denied_by_capture_policy(event: &span::Event, capture: &config::CaptureConfig) -> bool {
    if capture
        .deny_tools
        .iter()
        .any(|tool| tool == &event.tool_name)
    {
        return true;
    }
    if let Some(cwd) = &event.cwd
        && capture
            .deny_cwd_prefixes
            .iter()
            .any(|prefix| cwd.starts_with(prefix))
    {
        return true;
    }
    false
}

/// Recursively replaces base64 image data with a small marker, in place. Targets the
/// standard image content block (`{"type":"image","source":{"data":"…"}}`), any
/// `data` field holding a long base64 string, and any very long base64-looking
/// string anywhere — which is what a Computer Use screenshot looks like. The action
/// fields (`action`, `coordinate`, `text`, …) are untouched.
fn strip_screenshots(value: &mut serde_json::Value) {
    const DATA_MIN: usize = 1024; // a `data` field this long is a blob, not content
    const ANY_MIN: usize = 100_000; // any string this long is certainly a blob
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                let is_blob = v.as_str().is_some_and(|s| {
                    (key == "data" && s.len() >= DATA_MIN && looks_base64(s))
                        || (s.len() >= ANY_MIN && looks_base64(s))
                });
                if is_blob {
                    let bytes = v.as_str().map(str::len).unwrap_or(0);
                    *v = serde_json::json!(format!("[galdr stripped screenshot: {bytes} bytes]"));
                } else {
                    strip_screenshots(v);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                strip_screenshots(item);
            }
        }
        _ => {}
    }
}

/// A cheap base64 check: the string is non-trivial and made (almost) entirely of the
/// base64 alphabet. Good enough to tell an image blob from prose without decoding.
fn looks_base64(s: &str) -> bool {
    if s.len() < 64 {
        return false;
    }
    let ok = s
        .bytes()
        .filter(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'\n' | b'\r'))
        .count();
    ok as f64 / s.len() as f64 > 0.95
}

fn apply_response_cap(event: &mut span::Event, max_chars: Option<usize>) {
    let Some(max_chars) = max_chars else {
        return;
    };
    let raw = event.tool_response.to_string();
    if raw.chars().count() <= max_chars {
        return;
    }
    let preview: String = raw.chars().take(max_chars).collect();
    event.tool_response = serde_json::json!({
        "galdr_truncated": true,
        "original_chars": raw.chars().count(),
        "preview": preview,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::ActiveRec;

    #[test]
    fn strips_a_base64_screenshot_but_keeps_the_action() {
        let big = "iVBORw0KGgoAAAANSUhEUg".repeat(100); // long base64-looking blob
        let mut response = serde_json::json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": big }
        });
        let mut input = serde_json::json!({ "action": "screenshot" });
        strip_screenshots(&mut response);
        strip_screenshots(&mut input);
        // The pixels are gone, replaced by a marker.
        let data = response["source"]["data"].as_str().unwrap();
        assert!(data.contains("stripped screenshot"), "{data}");
        assert!(!data.contains("iVBORw0KGgo"));
        // The action is untouched.
        assert_eq!(input["action"], "screenshot");
    }

    #[test]
    fn strip_leaves_short_data_and_prose_alone() {
        // A short `data` value (e.g. a small payload) and ordinary prose are kept.
        let mut v = serde_json::json!({
            "data": "ok",
            "command": "git status",
            "note": "a normal sentence with spaces, not base64"
        });
        let before = v.clone();
        strip_screenshots(&mut v);
        assert_eq!(v, before);
    }

    fn active(origin: Option<&str>, bound: Option<&str>) -> ActiveRec {
        ActiveRec {
            rec_id: "01X".into(),
            name: "t".into(),
            started_at: "ts".into(),
            transcript_path: None,
            origin_cwd: origin.map(String::from),
            bound_session: bound.map(String::from),
        }
    }

    #[test]
    fn binds_to_the_first_session_under_origin() {
        let a = active(Some("/proj/galdr"), None);
        match capture_decision(&a, Some("sessA"), Some("/proj/galdr/sub")) {
            Capture::RecordAndBind(s) => assert_eq!(s, "sessA"),
            _ => panic!("should bind to sessA"),
        }
    }

    #[test]
    fn foreign_session_in_another_dir_is_skipped_before_binding() {
        let a = active(Some("/proj/galdr"), None);
        assert!(matches!(
            capture_decision(&a, Some("sessB"), Some("/proj/eldr")),
            Capture::Skip
        ));
    }

    #[test]
    fn once_bound_only_that_session_records() {
        let a = active(Some("/proj/galdr"), Some("sessA"));
        assert!(matches!(
            capture_decision(&a, Some("sessA"), Some("/anywhere")),
            Capture::Record
        ));
        assert!(matches!(
            capture_decision(&a, Some("sessB"), Some("/proj/galdr")),
            Capture::Skip
        ));
    }

    #[test]
    fn sessionless_events_always_record() {
        // Harnesses that omit session_id, and the no-scope tests, keep working.
        let unbound = active(Some("/proj/galdr"), None);
        assert!(matches!(
            capture_decision(&unbound, None, Some("/tmp")),
            Capture::Record
        ));
        let bound = active(Some("/proj/galdr"), Some("sessA"));
        assert!(matches!(
            capture_decision(&bound, None, None),
            Capture::Record
        ));
    }

    #[test]
    fn no_origin_binds_to_any_first_session() {
        let a = active(None, None);
        assert!(matches!(
            capture_decision(&a, Some("sessA"), Some("/anywhere")),
            Capture::RecordAndBind(_)
        ));
    }

    #[test]
    fn path_within_respects_component_boundaries() {
        assert!(path_within("/a/b", "/a/b"));
        assert!(path_within("/a/b/c", "/a/b"));
        assert!(!path_within("/a/bc", "/a/b"));
        assert!(!path_within("/x", "/a/b"));
    }
}
