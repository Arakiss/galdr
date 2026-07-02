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
    let route = route_event(&actives, input.session_id.as_deref(), input.cwd.as_deref());
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

    let capture = config::Config::load_capture();
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

/// Where the sensor routes one event.
enum Route {
    /// Drop it: no active recording owns this session, and none is free to claim it.
    Skip,
    /// Append it to `rec_id`; when `bind` is set, lock that recording onto the session.
    Record {
        rec_id: String,
        bind: Option<String>,
    },
}

/// Routes one event to at most one active recording, deterministically. `actives` is
/// newest-first, so "the most recent" is simply the first match.
///
/// 1. A recording already **bound** to this session gets the event.
/// 2. Otherwise, if the event carries a session id, the most recent **unbound**
///    recording eligible by `origin_cwd` is bound to this session and gets the event.
///    So the first real event after `galdr rec start` binds the recording to the
///    session that started it, and a session in another directory cannot claim it —
///    nor can a session that already owns a recording steal a second one (step 1 keeps
///    routing its events to the first, leaving the extra recording waiting unbound).
/// 3. A session-less event goes to the most recent active recording (harnesses that
///    omit the id, and the single-recording case, keep working).
/// 4. Anything else is dropped.
fn route_event(
    actives: &[record::ActiveRec],
    session_id: Option<&str>,
    cwd: Option<&str>,
) -> Route {
    let Some(sid) = session_id else {
        return match actives.first() {
            Some(a) => Route::Record {
                rec_id: a.rec_id.clone(),
                bind: None,
            },
            None => Route::Skip,
        };
    };
    if let Some(a) = actives
        .iter()
        .find(|a| a.bound_session.as_deref() == Some(sid))
    {
        return Route::Record {
            rec_id: a.rec_id.clone(),
            bind: None,
        };
    }
    if let Some(a) = actives
        .iter()
        .find(|a| a.bound_session.is_none() && cwd_ok(a.origin_cwd.as_deref(), cwd))
    {
        return Route::Record {
            rec_id: a.rec_id.clone(),
            bind: Some(sid.to_string()),
        };
    }
    Route::Skip
}

/// Whether an event's `cwd` may bind a recording opened in `origin`. No origin (or no
/// event cwd) means any directory is fine; otherwise the event must have happened at
/// or under the origin, compared by path component.
fn cwd_ok(origin: Option<&str>, cwd: Option<&str>) -> bool {
    match (origin, cwd) {
        (None, _) | (Some(_), None) => true,
        (Some(origin), Some(cwd)) => path_within(cwd, origin),
    }
}

/// True if `path` is `base` itself or lives under it, comparing path components so
/// `/a/bc` is not treated as under `/a/b`.
fn path_within(path: &str, base: &str) -> bool {
    let base = base.trim_end_matches('/');
    path == base || path.starts_with(&format!("{base}/"))
}

/// True if the event is a Bash call that does *nothing but* drive a galdr recording
/// (`galdr rec start/stop/status`, or the `galdr hook` sensor), possibly with a leading
/// `cd`. Such a call is instrumentation of the capture, not a step of the task.
///
/// It is deliberately conservative about compound commands: the call is a control
/// command only when **every** `&&`/`;`/`|`-separated segment is either a `cd` or a
/// `galdr rec/hook` invocation. If real work is bundled into the same call
/// (`galdr rec start x && cargo build`), the call is kept — recording galdr's own
/// command as a step is a far milder cost than silently dropping the bundled work.
/// The program word must be unquoted, so a control phrase quoted inside another
/// command (a commit message like `git commit -m 'galdr rec start'`) is still recorded.
fn is_galdr_control_command(tool_name: &str, tool_input: &serde_json::Value) -> bool {
    if tool_name != "Bash" {
        return false;
    }
    let Some(command) = tool_input.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    let segments: Vec<&str> = command
        .split([';', '\n', '|', '&'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    !segments.is_empty()
        && segments
            .iter()
            .all(|seg| is_cd_segment(seg) || is_galdr_rec_segment(seg))
}

/// A bare `cd` or `cd <dir>` segment.
fn is_cd_segment(seg: &str) -> bool {
    seg == "cd" || seg.starts_with("cd ")
}

/// A segment whose program is `galdr` (bare or by absolute path) and whose first
/// argument is `rec` or `hook`.
fn is_galdr_rec_segment(seg: &str) -> bool {
    let mut tokens = seg.split_whitespace();
    let Some(prog) = tokens.next() else {
        return false;
    };
    let is_galdr = prog == "galdr" || prog.ends_with("/galdr");
    is_galdr && matches!(tokens.next(), Some("rec") | Some("hook"))
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

/// Recursively replaces base64 image data with a small marker, in place. It strips
/// **only with image context** — never generic base64 — so it can't silently corrupt
/// an arbitrary tool's payload (spans are append-only; a wrong strip is irreversible).
/// Image context means: an `image` content block (`type: image`, or a `media_type` /
/// `mimeType` of `image/*`), an image-ish key (`image`, `image_url`, `screenshot`),
/// or a `data:image/…;base64,…` data URI. The standard Computer Use screenshot —
/// `{"type":"image","source":{"type":"base64","media_type":"image/png","data":"…"}}`
/// — is caught; the action fields (`action`, `coordinate`, `text`) are untouched.
fn strip_screenshots(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let image_ctx = is_image_context(map);
            for (key, v) in map.iter_mut() {
                let strip = match v.as_str() {
                    Some(s) if is_data_uri_image(s) => true,
                    Some(s) if (image_ctx || is_image_key(key)) && is_base64ish(s) => true,
                    _ => false,
                };
                if strip {
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

/// Whether this object is (or directly describes) an image content block.
fn is_image_context(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    let s = |k: &str| map.get(k).and_then(|v| v.as_str());
    s("type") == Some("image")
        || ["media_type", "mimeType", "mime_type"]
            .iter()
            .any(|k| s(k).is_some_and(|m| m.starts_with("image/")))
}

fn is_image_key(key: &str) -> bool {
    matches!(
        key,
        "image" | "image_url" | "imageUrl" | "screenshot" | "img"
    )
}

fn is_data_uri_image(s: &str) -> bool {
    s.starts_with("data:image/")
}

/// A cheap base64 check (base64 and base64url), long enough to be a real image, not a
/// short token. Only consulted once image context is already established.
fn is_base64ish(s: &str) -> bool {
    if s.len() < 64 {
        return false;
    }
    let ok = s
        .bytes()
        .filter(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_' | b'\n' | b'\r')
        })
        .count();
    ok as f64 / s.len() as f64 > 0.95
}

/// Saves any image blobs in this event as ephemeral PNG frames under
/// `~/.galdr/frames/<rec_id>/`. Best-effort and silent: a failed frame never affects
/// the recording (the span is the truth; frames are disposable authoring scaffolding).
fn save_frames(rec_id: &str, seq: u64, input: &serde_json::Value, response: &serde_json::Value) {
    let Ok(dir) = paths::frames_dir(rec_id) else {
        return;
    };
    write_image_blobs(&dir, seq, input, response);
}

/// Collects image blobs from `input`/`response`, decodes them, and writes one PNG per
/// blob to `dir` (`<seq>.png`, `<seq>-1.png`, …). Returns how many were written. Split
/// from [`save_frames`] so the path resolution is injectable in tests.
fn write_image_blobs(
    dir: &std::path::Path,
    seq: u64,
    input: &serde_json::Value,
    response: &serde_json::Value,
) -> usize {
    let mut blobs = Vec::new();
    collect_image_blobs(input, &mut blobs);
    collect_image_blobs(response, &mut blobs);
    if blobs.is_empty() {
        return 0;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return 0;
    }
    let mut written = 0;
    for (i, b64) in blobs.iter().enumerate() {
        let Some(bytes) = decode_base64(b64) else {
            continue;
        };
        let name = if i == 0 {
            format!("{seq:04}.png")
        } else {
            format!("{seq:04}-{i}.png")
        };
        if std::fs::write(dir.join(name), bytes).is_ok() {
            written += 1;
        }
    }
    written
}

/// Read-only twin of [`strip_screenshots`]: walks the value with the same image-context
/// rules and collects the base64 payloads (data-URI prefix removed) instead of replacing
/// them. Keeping the detection identical means a frame is saved for exactly what is
/// stripped — never an arbitrary string.
fn collect_image_blobs(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            let image_ctx = is_image_context(map);
            for (key, v) in map {
                match v.as_str() {
                    Some(s) if is_data_uri_image(s) => {
                        if let Some((_, b64)) = s.split_once(',') {
                            out.push(b64.to_string());
                        }
                    }
                    Some(s) if (image_ctx || is_image_key(key)) && is_base64ish(s) => {
                        out.push(s.to_string());
                    }
                    _ => collect_image_blobs(v, out),
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_image_blobs(item, out);
            }
        }
        _ => {}
    }
}

/// Decodes standard or URL-safe base64 (no external crate — fewer deps, no supply-chain
/// surface for a defensive local tool). Skips padding and whitespace; returns `None` on
/// any out-of-alphabet byte so a malformed blob writes no frame rather than garbage.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        buf = (buf << 6) | sextet(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Normalizes the tool result across harnesses. Claude Code and Codex send
/// `tool_response` (a JSON object); Cursor sends `tool_output` (the same payload, but
/// JSON-stringified). Prefer a present `tool_response`; otherwise adopt `tool_output`,
/// parsed back into JSON when it is a valid JSON string (else kept verbatim).
fn cursor_response(
    tool_response: serde_json::Value,
    tool_output: Option<serde_json::Value>,
) -> serde_json::Value {
    if !tool_response.is_null() {
        return tool_response;
    }
    match tool_output {
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s))
        }
        Some(other) => other,
        None => serde_json::Value::Null,
    }
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
    fn cursor_response_maps_tool_output_to_tool_response() {
        // Claude Code / Codex send `tool_response`; it wins when present.
        assert_eq!(
            cursor_response(
                serde_json::json!({ "exit_code": 0 }),
                Some(serde_json::json!("ignored"))
            ),
            serde_json::json!({ "exit_code": 0 })
        );
        // Cursor sends `tool_output` as a JSON-stringified string → parsed back to JSON.
        assert_eq!(
            cursor_response(
                serde_json::Value::Null,
                Some(serde_json::json!("{\"ok\":true}"))
            ),
            serde_json::json!({ "ok": true })
        );
        // A non-JSON tool_output string is kept verbatim rather than dropped.
        assert_eq!(
            cursor_response(serde_json::Value::Null, Some(serde_json::json!("plain"))),
            serde_json::json!("plain")
        );
        // Nothing present → null.
        assert_eq!(
            cursor_response(serde_json::Value::Null, None),
            serde_json::Value::Null
        );
    }

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
    fn decode_base64_matches_known_values() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        // padding optional, embedded whitespace skipped
        assert_eq!(decode_base64("aGVs\nbG8").unwrap(), b"hello");
        // an out-of-alphabet byte yields nothing rather than garbage
        assert!(decode_base64("abc!def").is_none());
    }

    #[test]
    fn keep_frames_writes_a_png_and_ignores_action_fields() {
        let b64 = "iVBORw0KGgoAAAANSUhEUg".repeat(4); // ≥64 base64-ish chars
        let response = serde_json::json!({
            "type": "image",
            "source": { "type": "base64", "media_type": "image/png", "data": b64 }
        });
        // Action fields are not images: they must produce no frame.
        let input = serde_json::json!({ "action": "screenshot", "coordinate": [10, 20] });

        let dir = std::env::temp_dir().join(format!("galdr-frames-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = write_image_blobs(&dir, 7, &input, &response);
        assert_eq!(
            written, 1,
            "exactly the one screenshot blob becomes a frame"
        );

        let png = dir.join("0007.png");
        assert!(png.exists(), "frame written at the seq-named path");
        assert_eq!(
            std::fs::read(&png).unwrap(),
            decode_base64(&b64).unwrap(),
            "the frame holds the decoded pixels"
        );
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn strip_only_acts_with_image_context_not_generic_base64() {
        let blob = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVowMTIzNDU2Nzg5".repeat(50);
        // No image context: a large base64-looking value under `stdout` is PRESERVED
        // (stripping it would be irreversible data loss for an arbitrary tool).
        let mut generic = serde_json::json!({ "stdout": blob });
        let before = generic.clone();
        strip_screenshots(&mut generic);
        assert_eq!(generic, before, "generic base64 must not be stripped");

        // A data: image URI is stripped regardless of context.
        let mut uri = serde_json::json!({ "url": format!("data:image/png;base64,{blob}") });
        strip_screenshots(&mut uri);
        assert!(uri["url"].as_str().unwrap().contains("stripped screenshot"));

        // An image-ish key with base64url payload is stripped.
        let mut keyed = serde_json::json!({ "image": blob.replace('+', "-") });
        strip_screenshots(&mut keyed);
        assert!(
            keyed["image"]
                .as_str()
                .unwrap()
                .contains("stripped screenshot")
        );
    }

    fn mk(rec_id: &str, origin: Option<&str>, bound: Option<&str>) -> ActiveRec {
        ActiveRec {
            rec_id: rec_id.into(),
            name: "t".into(),
            started_at: "ts".into(),
            transcript_path: None,
            origin_cwd: origin.map(String::from),
            bound_session: bound.map(String::from),
        }
    }

    /// Asserts the route records to `rec_id` and binds (or not) as expected.
    fn assert_records(route: Route, rec_id: &str, bind: Option<&str>) {
        match route {
            Route::Record {
                rec_id: got,
                bind: got_bind,
            } => {
                assert_eq!(got, rec_id, "wrong recording");
                assert_eq!(got_bind.as_deref(), bind, "wrong binding");
            }
            Route::Skip => panic!("expected a Record to {rec_id}, got Skip"),
        }
    }

    #[test]
    fn binds_to_the_first_session_under_origin() {
        let actives = [mk("01A", Some("/proj/galdr"), None)];
        assert_records(
            route_event(&actives, Some("sessA"), Some("/proj/galdr/sub")),
            "01A",
            Some("sessA"),
        );
    }

    #[test]
    fn foreign_session_in_another_dir_is_skipped_before_binding() {
        let actives = [mk("01A", Some("/proj/galdr"), None)];
        assert!(matches!(
            route_event(&actives, Some("sessB"), Some("/proj/eldr")),
            Route::Skip
        ));
    }

    #[test]
    fn once_bound_only_that_session_records() {
        let actives = [mk("01A", Some("/proj/galdr"), Some("sessA"))];
        // The bound session records without re-binding.
        assert_records(
            route_event(&actives, Some("sessA"), Some("/anywhere")),
            "01A",
            None,
        );
        // Another session cannot claim the already-bound recording.
        assert!(matches!(
            route_event(&actives, Some("sessB"), Some("/proj/galdr")),
            Route::Skip
        ));
    }

    #[test]
    fn sessionless_events_record_to_the_newest_active() {
        // Harnesses that omit session_id, and the single-recording case, keep working.
        let unbound = [mk("01A", Some("/proj/galdr"), None)];
        assert_records(route_event(&unbound, None, Some("/tmp")), "01A", None);
        let bound = [mk("01A", Some("/proj/galdr"), Some("sessA"))];
        assert_records(route_event(&bound, None, None), "01A", None);
        // With several active, a session-less event goes to the newest (01C).
        let many = [
            mk("01C", None, None),
            mk("01B", None, Some("sessB")),
            mk("01A", None, None),
        ];
        assert_records(route_event(&many, None, None), "01C", None);
    }

    #[test]
    fn no_origin_binds_to_any_first_session() {
        let actives = [mk("01A", None, None)];
        assert_records(
            route_event(&actives, Some("sessA"), Some("/anywhere")),
            "01A",
            Some("sessA"),
        );
    }

    #[test]
    fn routes_to_the_bound_recording_among_several() {
        // Two recordings, each bound to a different session: each session's events go
        // only to its own recording — no cross-contamination.
        let actives = [
            mk("01B", Some("/projB"), Some("sessB")),
            mk("01A", Some("/projA"), Some("sessA")),
        ];
        assert_records(
            route_event(&actives, Some("sessA"), Some("/projA")),
            "01A",
            None,
        );
        assert_records(
            route_event(&actives, Some("sessB"), Some("/projB")),
            "01B",
            None,
        );
        // A third, unknown session with no unbound recording to claim is dropped.
        assert!(matches!(
            route_event(&actives, Some("sessC"), Some("/projC")),
            Route::Skip
        ));
    }

    #[test]
    fn binds_the_most_recent_unbound_eligible_recording() {
        // Newest-first. Two unbound recordings the session's cwd matches: the most
        // recent (01C) is claimed; a same-cwd session then finds only 01B left.
        let actives = [
            mk("01C", Some("/proj"), None),
            mk("01B", Some("/proj"), None),
            mk("01A", Some("/other"), Some("old")),
        ];
        assert_records(
            route_event(&actives, Some("sessNew"), Some("/proj/x")),
            "01C",
            Some("sessNew"),
        );
    }

    #[test]
    fn unbound_recording_in_another_dir_is_not_claimed() {
        // The only unbound recording lives under a different origin: an event from
        // elsewhere must not bind it (no stealing across directories).
        let actives = [mk("01A", Some("/projA"), None)];
        assert!(matches!(
            route_event(&actives, Some("sessB"), Some("/projB")),
            Route::Skip
        ));
    }

    #[test]
    fn path_within_respects_component_boundaries() {
        assert!(path_within("/a/b", "/a/b"));
        assert!(path_within("/a/b/c", "/a/b"));
        assert!(!path_within("/a/bc", "/a/b"));
        assert!(!path_within("/x", "/a/b"));
    }

    #[test]
    fn galdr_control_commands_are_not_recorded() {
        let ctl =
            |cmd: &str| is_galdr_control_command("Bash", &serde_json::json!({ "command": cmd }));
        assert!(ctl("galdr rec start my-task"));
        assert!(ctl("galdr rec stop"));
        assert!(ctl("galdr rec status"));
        // Tolerates a leading cd and an absolute path to the binary.
        assert!(ctl("cd /repo && galdr rec start x"));
        assert!(ctl("/Users/me/.cargo/bin/galdr rec stop"));
        assert!(ctl("galdr rec start x >/dev/null"));
        // Real task commands — including other galdr subcommands — are recorded.
        assert!(!ctl("galdr distill 01ABC"));
        assert!(!ctl("cargo test"));
        // A control phrase quoted inside another command stays recorded.
        assert!(!ctl("git commit -m 'galdr rec start'"));
        // Real work bundled into the same call is kept, not dropped with the control cmd.
        assert!(!ctl("galdr rec start x && cargo build"));
        assert!(!ctl(
            "galdr rec start x\nVER=$(grep version Cargo.toml)\necho $VER"
        ));
        // Only Bash is inspected; an unrelated tool with such input is not a control cmd.
        assert!(!is_galdr_control_command(
            "Read",
            &serde_json::json!({ "command": "galdr rec start x" })
        ));
    }
}
