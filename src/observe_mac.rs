//! Native macOS human-observation lane.
//!
//! The third sensor after `rec` (agent tool calls) and `observe browser` (CDP).
//! `observe mac` records a human demonstration of a GUI workflow on the Mac: clicks,
//! scrolls and keystrokes captured through a `CGEventTap` in listen-only mode, written
//! as typed human span events (`human.mac.*`). It mirrors the browser lane's shape —
//! a session file, a spawned sensor process, an events NDJSON that `stop` folds into
//! the immutable span — so distillation, the catalog and the daemon treat it like any
//! other recording.
//!
//! Phase 1 captures the action and its coordinates only. The accessibility context of
//! the clicked element (role, title, window) and optional screenshots are later phases;
//! the wire format already carries the fields so adding them needs no migration.
//!
//! Everything that touches macOS frameworks lives behind `#[cfg(target_os = "macos")]`
//! in [`sensor`]. On other platforms the sensor is a stub that fails with a clear
//! message, exactly as `launchd` install does off macOS.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::span::{
    Event, EventKind, HumanAction, HumanEvent, HumanSource, HumanTarget, HumanValue, TargetLocator,
};
use crate::{catalog, ipc, paths, record, span, style};

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
    /// Virtual keycode for key events. The literal character is never captured in
    /// phase 1 — keystrokes are recorded as an occurrence, not their content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keycode: Option<i64>,
    /// Scroll delta (vertical) for scroll events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scroll_delta: Option<i64>,
}

/// Maps a wire event into a typed human span [`Event`]. Pure and total: it never fails,
/// so a malformed capture degrades to a bare action rather than dropping the step.
fn mac_wire_to_event(seq: u64, wire: MacWireEvent) -> Event {
    let source = HumanSource::MacApp {
        app: None,
        window_title: None,
    };

    // Coordinates travel as an element summary for now (debug metadata), never as the
    // primary locator: a skill that targets "the Save button" survives a resize; one
    // that targets (x, y) does not. The semantic locator lands with AX context.
    let coord_summary = match (wire.x, wire.y) {
        (Some(x), Some(y)) => Some(format!("screen ({x:.0}, {y:.0})")),
        _ => None,
    };
    let target = coord_summary.map(|summary| HumanTarget {
        primary: TargetLocator::Role {
            role: "AXUnknown".to_string(),
            name: None,
        },
        alternates: Vec::new(),
        role: None,
        name: None,
        text: None,
        label: None,
        placeholder: None,
        element_summary: Some(summary),
    });

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

fn count_event_lines(path: &std::path::Path) -> usize {
    match std::fs::read_to_string(path) {
        Ok(contents) => contents.lines().filter(|l| !l.trim().is_empty()).count(),
        Err(_) => 0,
    }
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
    if paths::mac_observe_active()?.exists() {
        bail!("a macOS observation is already active. Run `galdr observe mac stop` first.");
    }

    sensor::preflight()?;

    let rec_id = Ulid::new().to_string();
    let started_at = record::now_rfc3339();
    let session_dir = paths::mac_observe_session_dir(&rec_id)?;
    std::fs::create_dir_all(&session_dir)?;
    let events_file = session_dir.join("events.ndjson");
    let log_file = session_dir.join("sensor.log");
    let stop_flag = session_dir.join("stop");
    std::fs::write(&events_file, "")?;

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
    let event_count = count_event_lines(&session.events_file);
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

    // Ask the sensor to leave its run loop, then make sure it is gone.
    let _ = std::fs::write(&session.stop_flag, "1");
    stop_sensor(session.sensor_pid);

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
    };
    write_recording_files(&recording, &events)?;
    let _ = std::fs::remove_file(paths::mac_observe_active()?);
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
    let child = std::process::Command::new(exe)
        .args(["observe", "mac", "serve", rec_id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
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

// ── Native sensor ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod sensor {
    use std::ffi::c_void;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::Path;
    use std::ptr::NonNull;

    use anyhow::{Context, Result, bail};
    use objc2_core_foundation::{CFMachPort, CFRunLoop, Type, kCFRunLoopCommonModes};
    use objc2_core_graphics::{
        CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventTapProxy, CGEventType, CGPreflightListenEventAccess,
        CGRequestListenEventAccess,
    };

    use crate::record;

    /// State handed to the C event-tap callback through `user_info`.
    struct SensorCtx {
        log: std::fs::File,
        seq: u64,
        /// The tap's mach port, so the callback can re-enable it after the system
        /// disables it on a timeout. Raw because the callback reconstructs `&mut Ctx`.
        tap: *const CFMachPort,
    }

    /// Check the Input Monitoring permission without prompting. Called by `start` on the
    /// parent process so the user sees the guidance before a sensor is spawned.
    pub fn preflight() -> Result<()> {
        if CGPreflightListenEventAccess() {
            return Ok(());
        }
        // Trigger the one-time system prompt (adds galdr's host, disabled, to the list).
        let _ = CGRequestListenEventAccess();
        bail!(
            "galdr needs Input Monitoring to observe the Mac.\n  \
             Grant it in System Settings → Privacy & Security → Input Monitoring,\n  \
             enable the entry for your terminal (or galdr), then run `galdr observe mac start` again."
        );
    }

    /// Build the event-of-interest mask from a list of event types.
    fn mask_for(types: &[CGEventType]) -> CGEventMask {
        types.iter().fold(0u64, |acc, t| acc | (1u64 << t.0))
    }

    unsafe extern "C-unwind" fn callback(
        _proxy: CGEventTapProxy,
        etype: CGEventType,
        event: NonNull<CGEvent>,
        user_info: *mut c_void,
    ) -> *mut CGEvent {
        // SAFETY: `user_info` is the `&mut SensorCtx` we passed to `tap_create`, alive
        // for the whole run loop; the callback is single-threaded on the run loop.
        let ctx = unsafe { &mut *(user_info as *mut SensorCtx) };
        let ev = unsafe { event.as_ref() };

        // The system disables a slow or interrupted tap and notifies us; re-enable it
        // instead of going silently deaf.
        if etype == CGEventType::TapDisabledByTimeout
            || etype == CGEventType::TapDisabledByUserInput
        {
            if !ctx.tap.is_null() {
                // SAFETY: `ctx.tap` points at the CFMachPort owned by `run`, alive here.
                let tap = unsafe { &*ctx.tap };
                CGEvent::tap_enable(tap, true);
            }
            return event.as_ptr();
        }

        if let Some(wire) = describe(etype, ev)
            && let Ok(line) = serde_json::to_string(&wire)
            && writeln!(ctx.log, "{line}").is_ok()
        {
            ctx.seq += 1;
            let _ = ctx.log.flush();
        }

        // Listen-only: the return is ignored, but the contract is to pass the event on.
        event.as_ptr()
    }

    /// Build a wire event from one observed CGEvent, or `None` for events we ignore.
    /// Serialized through serde so the sensor never hand-rolls JSON.
    fn describe(etype: CGEventType, ev: &CGEvent) -> Option<super::MacWireEvent> {
        let point = CGEvent::location(Some(ev));
        let ts = record::now_rfc3339();
        let mut wire = super::MacWireEvent {
            ts,
            action: String::new(),
            x: Some(point.x),
            y: Some(point.y),
            button: None,
            keycode: None,
            scroll_delta: None,
        };

        match etype {
            CGEventType::LeftMouseDown => {
                wire.action = "human.mac.click".into();
                wire.button = Some(0);
            }
            CGEventType::RightMouseDown => {
                wire.action = "human.mac.click".into();
                wire.button = Some(1);
            }
            CGEventType::OtherMouseDown => {
                wire.action = "human.mac.click".into();
                wire.button = Some(CGEvent::integer_value_field(
                    Some(ev),
                    CGEventField::MouseEventButtonNumber,
                ));
            }
            CGEventType::KeyDown => {
                wire.action = "human.mac.key".into();
                wire.keycode = Some(CGEvent::integer_value_field(
                    Some(ev),
                    CGEventField::KeyboardEventKeycode,
                ));
            }
            CGEventType::ScrollWheel => {
                wire.action = "human.mac.scroll".into();
                wire.scroll_delta = Some(CGEvent::integer_value_field(
                    Some(ev),
                    CGEventField::ScrollWheelEventDeltaAxis1,
                ));
            }
            _ => return None,
        }
        Some(wire)
    }

    /// Install the tap and run its loop until the stop flag appears.
    pub fn run(events_file: &Path, stop_flag: &Path, log_file: &Path) -> Result<()> {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(events_file)
            .with_context(|| format!("could not open events file {}", events_file.display()))?;

        let mut ctx = Box::new(SensorCtx {
            log,
            seq: 0,
            tap: std::ptr::null(),
        });

        let types = [
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
            CGEventType::KeyDown,
            CGEventType::ScrollWheel,
        ];
        let mask = mask_for(&types);

        // SAFETY: the callback is implemented per its contract and `user_info` points at
        // `ctx`, which outlives the run loop (we hold `ctx` for the whole function).
        let tap = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::HIDEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                mask,
                Some(callback),
                (&mut *ctx as *mut SensorCtx) as *mut c_void,
            )
        };
        let Some(tap) = tap else {
            bail!(
                "could not create the event tap — Input Monitoring is likely not granted \
                 to this process"
            );
        };
        ctx.tap = &*tap as *const CFMachPort;

        let source = CFMachPort::new_run_loop_source(None, Some(&tap), 0)
            .context("could not create the run loop source for the event tap")?;
        let run_loop = CFRunLoop::current().context("no current run loop for the sensor")?;
        // SAFETY: FFI statics; the mode is a valid CFString for the process lifetime.
        let mode = unsafe { kCFRunLoopCommonModes };
        run_loop.add_source(Some(&source), mode);
        CGEvent::tap_enable(&tap, true);

        // A watcher stops the run loop when the stop flag appears. The run loop is
        // thread-bound, so we hand the watcher a thread-safe wake handle.
        spawn_stop_watcher(stop_flag.to_path_buf(), log_file.to_path_buf(), &run_loop);

        CFRunLoop::run();
        Ok(())
    }

    /// Poll for the stop flag on a helper thread and stop the (thread-bound) run loop.
    fn spawn_stop_watcher(
        stop_flag: std::path::PathBuf,
        _log: std::path::PathBuf,
        run_loop: &CFRunLoop,
    ) {
        // Retain the run loop so the pointer stays valid on the watcher thread.
        let handle = SendRunLoop(run_loop.retain());
        std::thread::spawn(move || {
            let handle = handle;
            loop {
                if stop_flag.exists() {
                    handle.0.stop();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        });
    }

    /// A `CFRunLoop` handle we assert is safe to move to the watcher thread: the only
    /// call made across it is `CFRunLoopStop`, which is documented thread-safe.
    struct SendRunLoop(objc2_core_foundation::CFRetained<CFRunLoop>);
    // SAFETY: CFRunLoopStop is explicitly safe to call from another thread.
    unsafe impl Send for SendRunLoop {}
}

#[cfg(not(target_os = "macos"))]
mod sensor {
    use std::path::Path;

    use anyhow::{Result, bail};

    pub fn preflight() -> Result<()> {
        bail!("`galdr observe mac` is only available on macOS.");
    }

    pub fn run(_events_file: &Path, _stop_flag: &Path, _log_file: &Path) -> Result<()> {
        bail!("`galdr observe mac` is only available on macOS.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_click_maps_to_human_click() {
        let wire = MacWireEvent {
            ts: "2026-07-02T00:00:00Z".into(),
            action: "human.mac.click".into(),
            x: Some(120.0),
            y: Some(48.0),
            button: Some(0),
            keycode: None,
            scroll_delta: None,
        };
        let event = mac_wire_to_event(0, wire);
        assert_eq!(event.event_kind, EventKind::Human);
        assert_eq!(event.tool_name, "human.mac.click");
        let human = event.human.expect("human payload");
        assert_eq!(human.action.as_str(), "human.mac.click");
        assert!(matches!(human.source, HumanSource::MacApp { .. }));
        let target = human.target.expect("coordinate summary target");
        assert_eq!(target.element_summary.as_deref(), Some("screen (120, 48)"));
        // A click carries no value: nothing typed, nothing to redact.
        assert!(human.value.is_none());
    }

    #[test]
    fn wire_key_omits_content() {
        let wire = MacWireEvent {
            ts: "2026-07-02T00:00:00Z".into(),
            action: "human.mac.key".into(),
            x: None,
            y: None,
            button: None,
            keycode: Some(9),
            scroll_delta: None,
        };
        let event = mac_wire_to_event(3, wire);
        assert_eq!(event.seq, 3);
        let human = event.human.expect("human payload");
        // The keystroke is recorded as an occurrence; its content is omitted by policy.
        match human.value {
            Some(HumanValue::Omitted { .. }) => {}
            other => panic!("expected omitted key value, got {other:?}"),
        }
    }

    #[test]
    fn wire_event_ndjson_roundtrips() {
        let wire = MacWireEvent {
            ts: "2026-07-02T00:00:00Z".into(),
            action: "human.mac.scroll".into(),
            x: Some(10.0),
            y: Some(20.0),
            button: None,
            keycode: None,
            scroll_delta: Some(-3),
        };
        let line = serde_json::to_string(&wire).unwrap();
        let back: MacWireEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(wire, back);
    }
}
