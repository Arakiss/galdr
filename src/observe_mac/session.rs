/// A macOS-observation session, serialized in `~/.galdr/observe/<rec_id>/session.json`
/// and pointed at by the active flag `~/.galdr/observe/mac-active.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MacObserveSession {
    rec_id: String,
    name: String,
    started_at: String,
    session_dir: PathBuf,
    events_file: PathBuf,
    log_file: PathBuf,
    stop_flag: PathBuf,
    cwd: Option<String>,
    #[serde(default)]
    sensor_pid: Option<u32>,
}

/// One human action as the native sensor writes it to the events NDJSON. Kept separate
/// from the span [`Event`] so the sensor's hot path serializes a tiny, stable record and
/// `stop` does the (fallible) mapping into the immutable span once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MacWireEvent {
    ts: String,
    /// Canonical action, e.g. `human.mac.click`, `human.mac.scroll`, `human.mac.key`.
    action: String,
    /// Screen coordinates at the time of the event (top-left origin). Debug metadata
    /// only — the replay is driven by semantic context, added in a later phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
    /// Mouse button number for click events (0 = left, 1 = right, 2 = other).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    button: Option<i64>,
    /// Scroll delta (vertical) for scroll events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scroll_delta: Option<i64>,
    /// Accessibility role of the element under the cursor at click time, e.g.
    /// `AXButton` (phase 2). Absent when the target app exposes no AX tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    /// Accessibility title/description of that element, e.g. the button's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Title of the window the element belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window: Option<String>,
    /// Owning application name, resolved from the element's pid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app: Option<String>,
}

/// Maps a wire event into a typed human span [`Event`]. Pure and total: it never fails,
/// so a malformed capture degrades to a bare action rather than dropping the step.
fn mac_wire_to_event(seq: u64, wire: MacWireEvent) -> Event {
    let source = HumanSource::MacApp {
        app: wire.app.clone(),
        window_title: wire.window.clone(),
    };

    // Coordinates travel as an element summary — debug metadata, never the primary
    // locator: a skill that targets "the Save button" survives a resize; one that
    // targets (x, y) does not.
    let coord_summary = match (wire.x, wire.y) {
        (Some(x), Some(y)) => Some(format!("screen ({x:.0}, {y:.0})")),
        _ => None,
    };
    // Build a semantic target when we resolved the accessibility role of the clicked
    // element; otherwise fall back to a coordinate-only target so the step is not lost.
    // A key event carries the mouse position too, but that coordinate is unrelated to the
    // keystroke, so a key with no resolved role gets no target rather than a bogus one.
    let is_key = wire.action == "human.mac.key";
    let target = if wire.role.is_some() || (coord_summary.is_some() && !is_key) {
        Some(HumanTarget {
            primary: TargetLocator::Role {
                role: wire.role.clone().unwrap_or_else(|| "AXUnknown".to_string()),
                name: wire.name.clone(),
            },
            alternates: Vec::new(),
            role: wire.role.clone(),
            name: wire.name.clone(),
            text: None,
            label: None,
            placeholder: None,
            element_summary: coord_summary,
        })
    } else {
        None
    };

    // Keystrokes record the fact of a key press, never the character: the value is
    // omitted by policy, so the raw span holds no typed content to leak.
    let value = if wire.action == "human.mac.key" {
        Some(HumanValue::Omitted {
            reason: "keystroke content not captured".to_string(),
        })
    } else {
        None
    };

    let human = HumanEvent {
        source,
        action: HumanAction::from(wire.action.as_str()),
        target,
        value,
        verification_hint: None,
        frame_ref: None,
    };

    Event {
        ts: wire.ts,
        seq,
        tool_name: wire.action,
        tool_input: serde_json::Value::Null,
        tool_response: serde_json::Value::Null,
        cwd: None,
        session_id: None,
        event_kind: EventKind::Human,
        human: Some(human),
    }
}

fn session_file(session_dir: &std::path::Path) -> PathBuf {
    session_dir.join("session.json")
}

fn write_session(session: &MacObserveSession) -> Result<()> {
    let path = session_file(&session.session_dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(session)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn write_active_pointer(session: &MacObserveSession) -> Result<()> {
    let path = paths::mac_observe_active()?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(session)?)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn read_active_session() -> Result<Option<MacObserveSession>> {
    let path = paths::mac_observe_active()?;
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    Ok(serde_json::from_str(&contents).ok())
}

fn read_session(rec_id: &str) -> Result<MacObserveSession> {
    let dir = paths::mac_observe_session_dir(rec_id)?;
    let contents = std::fs::read_to_string(session_file(&dir))
        .with_context(|| format!("no macOS observation session for {rec_id}"))?;
    Ok(serde_json::from_str(&contents)?)
}

fn read_wire_events(path: &std::path::Path) -> Result<Vec<MacWireEvent>> {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    let mut events = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<MacWireEvent>(line) {
            events.push(event);
        }
    }
    Ok(events)
}

/// `galdr observe mac start <name>` — begin a native macOS observation. Spawns a
/// detached sensor process that installs the event tap and appends wire events; this
/// call returns as soon as the sensor is running.
pub fn mac_start(name: String) -> Result<()> {
    paths::ensure_dirs()?;
    // A *parseable* active pointer means a live observation; refuse. A pointer that exists
    // but does not parse (schema drift, a manual edit) is stale: garbage-collect it so the
    // CLI never wedges — `start` refusing while `stop` sees nothing and orphans the sensor.
    if read_active_session()?.is_some() {
        bail!("a macOS observation is already active. Run `galdr observe mac stop` first.");
    }
    if let Ok(active) = paths::mac_observe_active()
        && active.exists()
    {
        let _ = std::fs::remove_file(&active);
    }

    sensor::preflight()?;
    if !sensor::accessibility_trusted() {
        eprintln!(
            "{} Accessibility is not granted — clicks will be recorded coordinate-only, with \
             no element role/name/window/app.\n  Grant it in System Settings → Privacy & \
             Security → Accessibility for semantic targeting.",
            style::amber("warning:")
        );
    }

    let rec_id = Ulid::new().to_string();
    let started_at = record::now_rfc3339();
    let session_dir = paths::mac_observe_session_dir(&rec_id)?;
    std::fs::create_dir_all(&session_dir)?;
    let events_file = session_dir.join("events.ndjson");
    let log_file = session_dir.join("sensor.log");
    let stop_flag = session_dir.join("stop");
    std::fs::write(&events_file, "")?;
    // The raw capture can hold pre-redaction context (window titles, app names); keep it
    // owner-only even though ~/.galdr is already 0700, and it is purged on stop.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&events_file, std::fs::Permissions::from_mode(0o600));
    }

    let mut session = MacObserveSession {
        rec_id: rec_id.clone(),
        name,
        started_at,
        session_dir,
        events_file,
        log_file,
        stop_flag,
        cwd: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
        sensor_pid: None,
    };
    write_session(&session)?;

    let sensor_pid = spawn_sensor(&rec_id)?;
    session.sensor_pid = Some(sensor_pid);
    write_session(&session)?;
    write_active_pointer(&session)?;

    println!(
        "{} observing macOS \"{}\"",
        style::accent("◆"),
        session.name
    );
    println!("  rec_id: {}", session.rec_id);
    println!("  sensor: CGEventTap (listen-only) — click · scroll · key");
    println!("  now perform the task; keystroke content is not captured");
    println!("  stop:  galdr observe mac stop");
    Ok(())
}

/// `galdr observe mac status` — show the active macOS observation, if any.
pub fn mac_status() -> Result<()> {
    let Some(session) = read_active_session()? else {
        println!("no active macOS observation");
        return Ok(());
    };
    // Count parsed events, the same rule `stop` folds into the span, so the two surfaces
    // never disagree on a torn or malformed trailing line.
    let event_count = read_wire_events(&session.events_file)
        .map(|e| e.len())
        .unwrap_or(0);
    let live = session.sensor_pid.map(sensor_alive).unwrap_or(false);
    println!("active macOS observation: {}", session.name);
    println!("  rec_id: {}", session.rec_id);
    println!("  events: {event_count}");
    println!("  sensor: {}", if live { "up" } else { "not running" });
    if let Some(pid) = session.sensor_pid {
        println!("  sensor_pid: {pid}");
    }
    Ok(())
}

/// `galdr observe mac stop` — end the active observation, fold its wire events into the
/// immutable span, and register the recording with the catalog and daemon.
pub fn mac_stop() -> Result<()> {
    let Some(session) = read_active_session()? else {
        println!("no active macOS observation");
        return Ok(());
    };

    // Ask the sensor to leave its run loop and give it a grace window to drain in-flight
    // hits and flush them. The watcher polls the flag every 150ms, then the run loop's
    // drop(ctx)+join drains the channel; only if the sensor is still alive after the grace
    // window do we SIGTERM as a true backstop. Reading events before it exits would fold
    // only the already-flushed prefix into the span and silently drop the tail.
    let _ = std::fs::write(&session.stop_flag, "1");
    if let Some(pid) = session.sensor_pid {
        let deadline = 40; // ~2s at 50ms steps
        let mut waited = 0;
        while waited < deadline && sensor_alive(pid) {
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 1;
        }
        if sensor_alive(pid) {
            stop_sensor(Some(pid));
        }
    }

    let wire_events = read_wire_events(&session.events_file)?;
    let events: Vec<Event> = wire_events
        .into_iter()
        .enumerate()
        .map(|(idx, wire)| mac_wire_to_event(idx as u64, wire))
        .collect();

    let recording = record::Recording {
        rec_id: session.rec_id.clone(),
        name: session.name.clone(),
        started_at: session.started_at.clone(),
        ended_at: record::now_rfc3339(),
        steps: events.len(),
        cwd: session.cwd.clone(),
        closed_reason: None,
    };
    write_recording_files(&recording, &events)?;
    let _ = std::fs::remove_file(paths::mac_observe_active()?);
    // Purge the raw capture now that it is folded into the immutable span. The span is the
    // canonical artifact (and gets redacted again at skill-install time); the intermediate
    // events file can hold pre-redaction context, so it must not linger on disk.
    let _ = std::fs::remove_dir_all(&session.session_dir);
    let _ = catalog::sync_closed_recording(&recording, &events);
    for event in &events {
        ipc::notify_best_effort(&ipc::Request::EventAppended {
            rec_id: recording.rec_id.clone(),
            event: Box::new(event.clone()),
        });
    }
    ipc::notify_best_effort(&ipc::Request::RecordingClosed {
        recording: recording.clone(),
    });

    println!(
        "{} stopped macOS observation \"{}\" — {} human steps",
        style::accent("■"),
        recording.name,
        events.len()
    );
    println!("  rec_id: {}", recording.rec_id);
    println!(
        "  turn it into a skill:  galdr distill {}",
        recording.rec_id
    );
    Ok(())
}

/// `galdr observe mac serve <rec_id>` (hidden) — the sensor process. Installs the event
/// tap and runs until the stop flag appears. Kept as a subcommand so `start` can spawn
/// it detached with its own run loop, exactly as the browser lane spawns its collector.
pub fn mac_serve(rec_id: &str) -> Result<()> {
    let session = read_session(rec_id)?;
    sensor::run(&session.events_file, &session.stop_flag, &session.log_file)
}

/// The two TCC permissions the native lane needs, checked without prompting. `None` off
/// macOS, where the lane does not apply. Consumed by `galdr doctor`.
pub struct MacPermissions {
    /// Input Monitoring — required for the event tap to receive keys.
    pub input_monitoring: bool,
    /// Accessibility — required to resolve the clicked element's role/name/window/app.
    pub accessibility: bool,
}

#[cfg(target_os = "macos")]
pub fn mac_permissions() -> Option<MacPermissions> {
    Some(MacPermissions {
        input_monitoring: sensor::input_monitoring_trusted(),
        accessibility: sensor::accessibility_trusted(),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn mac_permissions() -> Option<MacPermissions> {
    None
}

fn write_recording_files(recording: &record::Recording, events: &[Event]) -> Result<()> {
    let span_path = paths::span_file(&recording.rec_id)?;
    let rec_path = paths::recording_file(&recording.rec_id)?;
    if span_path.exists() || rec_path.exists() {
        bail!("recording id collision: {}", recording.rec_id);
    }

    let span_tmp = span_path.with_extension("jsonl.tmp");
    let rec_tmp = rec_path.with_extension("json.tmp");
    let mut span_jsonl = String::new();
    for event in events {
        span_jsonl.push_str(&serde_json::to_string(event)?);
        span_jsonl.push('\n');
    }
    std::fs::write(&span_tmp, span_jsonl)
        .with_context(|| format!("could not write temporary span {}", span_tmp.display()))?;
    std::fs::rename(&span_tmp, &span_path)
        .with_context(|| format!("could not publish span {}", span_path.display()))?;
    let _ = span::fsync(&span_path);

    std::fs::write(&rec_tmp, serde_json::to_string_pretty(recording)?)
        .with_context(|| format!("could not write temporary recording {}", rec_tmp.display()))?;
    std::fs::rename(&rec_tmp, &rec_path)
        .with_context(|| format!("could not publish recording {}", rec_path.display()))?;
    Ok(())
}

fn spawn_sensor(rec_id: &str) -> Result<u32> {
    let exe = std::env::current_exe().context("could not resolve the galdr binary path")?;
    // Route the sensor's stderr to its log file, not /dev/null, so a failure inside the
    // detached process (e.g. the tap could not be created because a permission was revoked
    // between preflight and spawn) leaves a diagnosable trace instead of vanishing.
    let log_path = paths::mac_observe_session_dir(rec_id)?.join("sensor.log");
    let stderr = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map(std::process::Stdio::from)
        .unwrap_or_else(|_| std::process::Stdio::null());
    let child = std::process::Command::new(exe)
        .args(["observe", "mac", "serve", rec_id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .spawn()
        .context("could not spawn the macOS observe sensor")?;
    Ok(child.id())
}

/// SIGTERM the sensor process (best-effort). The stop flag already asked it to leave its
/// run loop; this is the backstop for a sensor that never noticed the flag.
fn stop_sensor(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(unix)]
    {
        // SAFETY: `kill(2)` with a plain signal is a defined libc call; a stale pid
        // simply returns an error we ignore.
        unsafe {
            libc_kill(pid as i32, 15);
        }
    }
    let _ = pid;
}

fn sensor_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // `kill(pid, 0)` probes for existence without delivering a signal.
        // SAFETY: signal 0 is the standard existence check.
        unsafe { libc_kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}
