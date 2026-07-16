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
    /// Cursor names the result field `tool_output` (a JSON-stringified string), not
    /// `tool_response`; mapped over when the latter is absent.
    #[serde(default)]
    tool_output: Option<serde_json::Value>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    /// Cursor names the session `conversation_id`, not `session_id`.
    #[serde(default)]
    conversation_id: Option<String>,
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

    // No active recording anywhere: nothing to do, fast exit. Reading the active set
    // is O(active recordings) — a directory listing, no locks — so PostToolUse stays
    // fast even with several concurrent sessions recording.
    let actives = record::read_active_all();
    if actives.is_empty() {
        return Ok(());
    }

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

    // galdr's own recording-control commands (`galdr rec start/stop/status`, and the
    // `galdr hook` sensor itself) are instrumentation of the capture, not steps of the
    // task. Never record them: they would pollute the span, skew `diff` step counts,
    // and surface as a bogus parameter. This runs before session binding too, so the
    // first *real* event is the one that binds. (Task noise — temp reads, polling — is
    // filtered later at distill; this is specifically galdr's own meta-commands.)
    if is_galdr_control_command(&input.tool_name, &input.tool_input) {
        return Ok(());
    }

    // Route the event to at most one recording: the recording bound to this session
    // if any; else bind the most recent unbound recording eligible by `origin_cwd`;
    // else drop it. This keeps concurrent sessions' spans separate, so a session in
    // another project never leaks its tool calls (and payloads) into another's span.
    // Stale recordings (a forgotten `rec stop`) neither take session-less events nor
    // bind new sessions — only their own bound session still reaches them.
    let capture = config::Config::load_capture();
    let route = route_event(
        &actives,
        input.session_id.as_deref(),
        input.cwd.as_deref(),
        |a| record::is_stale(a, capture.stale_after_hours),
    );
    let Route::Record { rec_id, bind } = route else {
        return Ok(());
    };
    // The chosen recording's current flag, for its transcript/binding fields.
    let active = actives
        .into_iter()
        .find(|a| a.rec_id == rec_id)
        .expect("routed rec_id is one of the active recordings");

    let span_path = paths::span_file(&active.rec_id)?;
    let mut event = span::Event {
        ts: record::now_rfc3339(),
        seq: span::count_events(&span_path),
        tool_name: input.tool_name,
        tool_input: input.tool_input,
        // Cursor's `postToolUse` renames these two fields; map them so one `galdr hook`
        // command records every harness (Claude Code, Codex, Cursor) unchanged.
        tool_response: cursor_response(input.tool_response, input.tool_output),
        cwd: input.cwd,
        session_id: input.session_id.or(input.conversation_id),
        event_kind: span::EventKind::ToolCall,
        human: None,
    };

    if denied_by_capture_policy(&event, &capture) {
        return Ok(());
    }
    // Opt-in: keep the pixels as ephemeral authoring frames *before* they are stripped
    // from the span. They never enter the span; they are vision scaffolding for distill.
    if capture.keep_frames {
        save_frames(
            &active.rec_id,
            event.seq,
            &event.tool_input,
            &event.tool_response,
        );
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
        event: Box::new(event.clone()),
    });

    // Provenance seam: the core records nothing.
    ext::NoopExt.record(&event);

    // Persist any new session binding and capture the transcript once, in a single
    // write of this recording's active flag.
    let new_binding = bind;
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
