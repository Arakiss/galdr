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
use crate::{ext, ipc, paths, record, span};

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

    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        return Ok(());
    }
    let input: HookInput = serde_json::from_str(&buf)?;

    let span_path = paths::span_file(&active.rec_id)?;
    let event = span::Event {
        ts: record::now_rfc3339(),
        seq: span::count_events(&span_path),
        tool_name: input.tool_name,
        tool_input: input.tool_input,
        tool_response: input.tool_response,
        cwd: input.cwd,
        session_id: input.session_id,
    };

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

    // Capture the session transcript once, from the first event.
    if active.transcript_path.is_none()
        && let Some(transcript_path) = input.transcript_path
    {
        let updated = record::ActiveRec {
            transcript_path: Some(transcript_path),
            ..active
        };
        let _ = record::write_active(&updated);
    }
    Ok(())
}
