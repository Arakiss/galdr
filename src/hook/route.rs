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
///
/// `is_stale` marks recordings whose owner went silent (a forgotten `rec stop`).
/// A stale recording is invisible to rules 2 and 3 — it must not swallow
/// session-less events nor claim a new session — but rule 1 still applies: the
/// bound session's own activity always records, and the append refreshes the span's
/// mtime, so a recording whose session comes back to life un-stales itself.
fn route_event(
    actives: &[record::ActiveRec],
    session_id: Option<&str>,
    cwd: Option<&str>,
    is_stale: impl Fn(&record::ActiveRec) -> bool,
) -> Route {
    let Some(sid) = session_id else {
        return match actives.iter().find(|a| !is_stale(a)) {
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
        .find(|a| a.bound_session.is_none() && !is_stale(a) && cwd_ok(a.origin_cwd.as_deref(), cwd))
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
