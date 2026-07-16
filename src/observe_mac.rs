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

// ── Native sensor ────────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod sensor {
    use std::ffi::{CStr, c_char, c_float, c_void};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::Path;
    use std::ptr::NonNull;
    use std::sync::mpsc::{self, Receiver, Sender};

    use anyhow::{Context, Result, bail};
    use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement};
    use objc2_core_foundation::{
        CFMachPort, CFRetained, CFRunLoop, CFString, CFType, Type, kCFRunLoopCommonModes,
    };
    use objc2_core_graphics::{
        CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventTapProxy, CGEventType, CGPreflightListenEventAccess,
        CGRequestListenEventAccess,
    };

    use crate::record;

    /// AX messaging timeout for the sensor's queries (seconds). Kept well under the 6s
    /// default so a slow or unresponsive app never stalls context resolution.
    const AX_TIMEOUT_SECS: c_float = 0.12;

    /// A raw observed hit, sent from the tap callback to the resolver thread. Building it
    /// is the only work the callback does; the (slow, cross-process) AX resolution runs
    /// off the tap thread so a slow app can never stall — and thus disable — the tap.
    struct RawHit {
        ts: String,
        action: &'static str,
        x: f64,
        y: f64,
        button: Option<i64>,
        scroll_delta: Option<i64>,
        /// Resolve the accessibility context under (x, y)? True only for clicks.
        resolve_ax: bool,
    }

    /// State handed to the C event-tap callback through `user_info`.
    struct SensorCtx {
        tx: Sender<RawHit>,
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

    /// Whether Input Monitoring is granted, checked WITHOUT prompting (unlike `preflight`,
    /// which prompts and bails). For read-only status surfaces like `doctor`.
    pub fn input_monitoring_trusted() -> bool {
        CGPreflightListenEventAccess()
    }

    /// Whether the process may query the accessibility tree (the separate Accessibility
    /// TCC grant, distinct from Input Monitoring). Without it clicks still record, but
    /// coordinate-only — no role/name/window/app — so `start` warns rather than failing.
    pub fn accessibility_trusted() -> bool {
        // SAFETY: a parameterless predicate that only reads the TCC trust state.
        unsafe { AXIsProcessTrusted() }
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

        // Privacy hard gate: capture NOTHING while secure input is active — a credential
        // dialog is focused. macOS already blocks the tap from seeing keys
        // (EnableSecureEventInput), but clicks and AX resolution would otherwise still run
        // and record the role/label/window/app of a password sheet. Suppressing the whole
        // path keeps every kind of event out of the recording during a secure session, and
        // fails safe if a later phase adds key-up/modifier events to the mask.
        if secure_input_active() {
            return event.as_ptr();
        }

        // Minimal hot-path work: read the cheap in-process fields and hand the hit to the
        // resolver thread. No AX call here — that IPC belongs off the tap thread.
        if let Some(hit) = describe(etype, ev) {
            let _ = ctx.tx.send(hit);
        }

        // Listen-only: the return is ignored, but the contract is to pass the event on.
        event.as_ptr()
    }

    /// Whether any process currently has secure keyboard entry enabled — i.e. a password
    /// field is focused. Carbon's `IsSecureEventInputEnabled` is a global flag, exactly
    /// what TextExpander and Keyboard Maestro poll to suspend capture. Not bound by objc2,
    /// so we declare it and link the Carbon framework.
    fn secure_input_active() -> bool {
        #[link(name = "Carbon", kind = "framework")]
        unsafe extern "C" {
            fn IsSecureEventInputEnabled() -> u8;
        }
        // SAFETY: a parameterless Carbon predicate that only reads a global flag.
        unsafe { IsSecureEventInputEnabled() != 0 }
    }

    /// Build a raw hit from one observed CGEvent, or `None` for events we ignore.
    fn describe(etype: CGEventType, ev: &CGEvent) -> Option<RawHit> {
        let point = CGEvent::location(Some(ev));
        let mut hit = RawHit {
            ts: record::now_rfc3339(),
            action: "",
            x: point.x,
            y: point.y,
            button: None,
            scroll_delta: None,
            resolve_ax: false,
        };

        match etype {
            CGEventType::LeftMouseDown => {
                hit.action = "human.mac.click";
                hit.button = Some(0);
                hit.resolve_ax = true;
            }
            CGEventType::RightMouseDown => {
                hit.action = "human.mac.click";
                hit.button = Some(1);
                hit.resolve_ax = true;
            }
            CGEventType::OtherMouseDown => {
                hit.action = "human.mac.click";
                hit.button = Some(CGEvent::integer_value_field(
                    Some(ev),
                    CGEventField::MouseEventButtonNumber,
                ));
                hit.resolve_ax = true;
            }
            CGEventType::KeyDown => {
                // A key is recorded as an occurrence only. The virtual keycode is NEVER
                // read: it maps deterministically back to the typed character (that is how
                // keyloggers work), so capturing it — even to an intermediate file — would
                // defeat the "content is never captured" guarantee.
                hit.action = "human.mac.key";
            }
            CGEventType::ScrollWheel => {
                hit.action = "human.mac.scroll";
                hit.scroll_delta = Some(CGEvent::integer_value_field(
                    Some(ev),
                    CGEventField::ScrollWheelEventDeltaAxis1,
                ));
            }
            _ => return None,
        }
        Some(hit)
    }

    /// Install the tap and run its loop until the stop flag appears. AX resolution runs
    /// on a separate thread fed by the callback, so the tap thread stays fast.
    pub fn run(events_file: &Path, stop_flag: &Path, log_file: &Path) -> Result<()> {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(events_file)
            .with_context(|| format!("could not open events file {}", events_file.display()))?;

        let (tx, rx) = mpsc::channel::<RawHit>();
        let resolver = std::thread::spawn(move || resolve_loop(rx, log));

        // Own the context via a raw pointer for the whole run loop. The FFI idiom is
        // deliberate: `user_info` and every `ctx.tap` write go through this single raw
        // pointer, never through a `Box`/`&mut` owner — a reborrow-then-owner-access would
        // invalidate the pointer's provenance (Stacked/Tree Borrows) and make the
        // callback's deref UB. We reclaim and drop it after the run loop returns.
        let ctx_ptr = Box::into_raw(Box::new(SensorCtx {
            tx,
            tap: std::ptr::null(),
        }));

        let types = [
            CGEventType::LeftMouseDown,
            CGEventType::RightMouseDown,
            CGEventType::OtherMouseDown,
            CGEventType::KeyDown,
            CGEventType::ScrollWheel,
        ];
        let mask = mask_for(&types);

        // SAFETY: the callback is implemented per its contract and `user_info` points at
        // the leaked `SensorCtx`, which we keep alive until after the run loop returns.
        let tap = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::HIDEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                mask,
                Some(callback),
                ctx_ptr as *mut c_void,
            )
        };
        let Some(tap) = tap else {
            // Reclaim the leaked context on the error path so it is not leaked for real.
            drop(unsafe { Box::from_raw(ctx_ptr) });
            bail!(
                "could not create the event tap — Input Monitoring is likely not granted \
                 to this process"
            );
        };
        // SAFETY: no callback can fire yet (the tap is not enabled / not on a run loop),
        // and the write goes through the same raw provenance as `user_info`.
        unsafe {
            (*ctx_ptr).tap = &*tap as *const CFMachPort;
        }

        let source = CFMachPort::new_run_loop_source(None, Some(&tap), 0)
            .context("could not create the run loop source for the event tap")?;
        let run_loop = CFRunLoop::current().context("no current run loop for the sensor")?;
        // SAFETY: FFI statics; the mode is a valid CFString for the process lifetime.
        let mode = unsafe { kCFRunLoopCommonModes };
        run_loop.add_source(Some(&source), mode);
        CGEvent::tap_enable(&tap, true);

        // A watcher stops the run loop when the stop flag appears and re-arms the tap
        // if the system disabled it. The run loop is thread-bound, so we hand the
        // watcher retained, Send-asserted handles rather than borrows.
        let _ = log_file;
        spawn_watcher(stop_flag.to_path_buf(), &run_loop, &tap);

        CFRunLoop::run();

        // The run loop has stopped and no callback can run again. Reclaim the context and
        // drop it (dropping its Sender) so the resolver drains the remaining hits and
        // exits, then join it so every event is flushed to disk before `run` returns.
        // SAFETY: the run loop is done; the callback will not deref `ctx_ptr` again.
        drop(unsafe { Box::from_raw(ctx_ptr) });
        let _ = resolver.join();
        Ok(())
    }

    /// The AX-resolution loop: for each hit, enrich clicks with the accessibility context
    /// under the cursor, then serialize the wire event to disk. Runs off the tap thread.
    fn resolve_loop(rx: Receiver<RawHit>, mut log: std::fs::File) {
        // The system-wide element is created and used only on this thread (AXUIElement is
        // not thread-safe). Lowering its messaging timeout is process-global for us.
        let system = unsafe { AXUIElement::new_system_wide() };
        unsafe {
            let _ = system.set_messaging_timeout(AX_TIMEOUT_SECS);
        }

        for hit in rx {
            let mut wire = super::MacWireEvent {
                ts: hit.ts,
                action: hit.action.to_string(),
                x: Some(hit.x),
                y: Some(hit.y),
                button: hit.button,
                scroll_delta: hit.scroll_delta,
                role: None,
                name: None,
                window: None,
                app: None,
            };
            if hit.resolve_ax
                && let Some(cx) = unsafe { resolve_ax(&system, hit.x, hit.y) }
            {
                wire.role = cx.role;
                wire.name = cx.name;
                wire.window = cx.window;
                wire.app = cx.app;
            }
            if let Ok(line) = serde_json::to_string(&wire)
                && writeln!(log, "{line}").is_ok()
            {
                let _ = log.flush();
            }
        }
    }

    #[derive(Default)]
    struct AxContext {
        role: Option<String>,
        name: Option<String>,
        window: Option<String>,
        app: Option<String>,
    }

    /// Resolve the accessibility context of the element at screen coordinates (x, y).
    /// Coordinates are top-left origin, which is what both CGEvent and AX use — no flip.
    /// Best-effort: any failed query leaves that field `None` rather than dropping the step.
    ///
    /// # Safety
    /// `system` must be the system-wide AX element, used only on this thread.
    unsafe fn resolve_ax(system: &AXUIElement, x: f64, y: f64) -> Option<AxContext> {
        let mut el_ptr: *const AXUIElement = std::ptr::null();
        let err = unsafe {
            system.copy_element_at_position(x as c_float, y as c_float, NonNull::from(&mut el_ptr))
        };
        if err != AXError::Success {
            return None;
        }
        // SAFETY: on Success the out-param is a +1 retained AX element we now own.
        let el = unsafe { CFRetained::from_raw(NonNull::new(el_ptr as *mut AXUIElement)?) };

        let mut cx = AxContext {
            role: unsafe { copy_string_attr(&el, "AXRole") },
            name: unsafe { copy_string_attr(&el, "AXTitle") }
                .or_else(|| unsafe { copy_string_attr(&el, "AXDescription") }),
            ..Default::default()
        };
        if let Some(window) = unsafe { copy_element_attr(&el, "AXWindow") } {
            cx.window = unsafe { copy_string_attr(&window, "AXTitle") };
        }
        let mut pid: i32 = 0;
        if unsafe { el.pid(NonNull::from(&mut pid)) } == AXError::Success && pid > 0 {
            cx.app = app_name_for_pid(pid);
        }
        Some(cx)
    }

    /// Read a string-valued AX attribute (e.g. `"AXRole"`). The attribute-name strings are
    /// the stable public constant values (`kAXRoleAttribute == "AXRole"`); objc2 does not
    /// bind the CFString constants, so we spell them.
    ///
    /// # Safety
    /// `el` must be a live AX element used only on the resolver thread.
    unsafe fn copy_string_attr(el: &AXUIElement, attribute: &str) -> Option<String> {
        let attr = CFString::from_str(attribute);
        let mut value: *const CFType = std::ptr::null();
        let err = unsafe { el.copy_attribute_value(&attr, NonNull::from(&mut value)) };
        if err != AXError::Success {
            return None;
        }
        // SAFETY: on Success the out-param is a +1 retained CF value we now own.
        let value = unsafe { CFRetained::from_raw(NonNull::new(value as *mut CFType)?) };
        let s = value.downcast_ref::<CFString>()?;
        Some(s.to_string())
    }

    /// Read an AX attribute whose value is itself an AX element (e.g. `"AXWindow"`).
    ///
    /// # Safety
    /// `el` must be a live AX element used only on the resolver thread.
    unsafe fn copy_element_attr(
        el: &AXUIElement,
        attribute: &str,
    ) -> Option<CFRetained<AXUIElement>> {
        let attr = CFString::from_str(attribute);
        let mut value: *const CFType = std::ptr::null();
        let err = unsafe { el.copy_attribute_value(&attr, NonNull::from(&mut value)) };
        if err != AXError::Success {
            return None;
        }
        // SAFETY: on Success the out-param is a +1 retained AX element we now own.
        let value = unsafe { CFRetained::from_raw(NonNull::new(value as *mut CFType)?) };
        value.downcast::<AXUIElement>().ok()
    }

    /// Resolve a process name from its pid via `proc_name` (libproc, part of libSystem).
    fn app_name_for_pid(pid: i32) -> Option<String> {
        // `proc_name` copies the last path component of the executable, NUL-terminated.
        unsafe extern "C" {
            fn proc_name(pid: i32, buffer: *mut c_char, buffersize: u32) -> i32;
        }
        let mut buf = [0u8; 256];
        let n = unsafe { proc_name(pid, buf.as_mut_ptr() as *mut c_char, buf.len() as u32) };
        if n <= 0 {
            return None;
        }
        CStr::from_bytes_until_nul(&buf)
            .ok()
            .and_then(|c| c.to_str().ok())
            .map(str::to_string)
    }

    /// Watch, on a helper thread: stop the (thread-bound) run loop when the stop flag
    /// appears, and re-enable the tap if it went quiet. The callback already re-arms on
    /// `TapDisabledByTimeout`, but a tap can also die across sleep/wake *without* firing
    /// that callback — robust taps poll `CGEventTapIsEnabled` on a timer. This is that
    /// poll, folded into the stop watcher we already run.
    fn spawn_watcher(
        stop_flag: std::path::PathBuf,
        run_loop: &CFRunLoop,
        tap: &CFRetained<CFMachPort>,
    ) {
        // Retain both so the pointers stay valid on the watcher thread.
        let run_loop = SendRunLoop(run_loop.retain());
        let tap = SendTap(tap.retain());
        std::thread::spawn(move || {
            let run_loop = run_loop;
            let tap = tap;
            loop {
                if stop_flag.exists() {
                    run_loop.0.stop();
                    break;
                }
                if !CGEvent::tap_is_enabled(&tap.0) {
                    CGEvent::tap_enable(&tap.0, true);
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        });
    }

    /// A `CFRunLoop` handle we assert is safe to move to the watcher thread: the only
    /// call made across it is `CFRunLoopStop`, which is documented thread-safe.
    struct SendRunLoop(CFRetained<CFRunLoop>);
    // SAFETY: CFRunLoopStop is explicitly safe to call from another thread.
    unsafe impl Send for SendRunLoop {}

    /// A tap handle we assert is safe to move to the watcher thread: the only calls made
    /// across it are `CGEventTapIsEnabled`/`CGEventTapEnable`, thin toggles of the tap's
    /// window-server state that watchdog timers in other apps call the same way.
    struct SendTap(CFRetained<CFMachPort>);
    // SAFETY: the tap enable/is-enabled toggles are safe to call off the run loop thread.
    unsafe impl Send for SendTap {}
}

#[cfg(not(target_os = "macos"))]
mod sensor {
    use std::path::Path;

    use anyhow::{Result, bail};

    pub fn preflight() -> Result<()> {
        bail!("`galdr observe mac` is only available on macOS.");
    }

    pub fn accessibility_trusted() -> bool {
        false
    }

    pub fn run(_events_file: &Path, _stop_flag: &Path, _log_file: &Path) -> Result<()> {
        bail!("`galdr observe mac` is only available on macOS.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire(action: &str) -> MacWireEvent {
        MacWireEvent {
            ts: "2026-07-02T00:00:00Z".into(),
            action: action.into(),
            x: Some(120.0),
            y: Some(48.0),
            button: None,
            scroll_delta: None,
            role: None,
            name: None,
            window: None,
            app: None,
        }
    }

    #[test]
    fn wire_click_maps_to_human_click() {
        let mut wire = sample_wire("human.mac.click");
        wire.button = Some(0);
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
    fn wire_click_with_ax_builds_semantic_target() {
        let mut wire = sample_wire("human.mac.click");
        wire.role = Some("AXButton".into());
        wire.name = Some("Enviar".into());
        wire.window = Some("Gastos".into());
        wire.app = Some("Contsimple".into());
        let event = mac_wire_to_event(0, wire);
        let human = event.human.expect("human payload");
        // The app and window ride on the source; the role/name become the locator.
        match human.source {
            HumanSource::MacApp { app, window_title } => {
                assert_eq!(app.as_deref(), Some("Contsimple"));
                assert_eq!(window_title.as_deref(), Some("Gastos"));
            }
            other => panic!("expected MacApp source, got {other:?}"),
        }
        let target = human.target.expect("semantic target");
        assert_eq!(target.role.as_deref(), Some("AXButton"));
        assert_eq!(target.name.as_deref(), Some("Enviar"));
        match target.primary {
            TargetLocator::Role { role, name } => {
                assert_eq!(role, "AXButton");
                assert_eq!(name.as_deref(), Some("Enviar"));
            }
            other => panic!("expected a role locator, got {other:?}"),
        }
    }

    #[test]
    fn wire_key_omits_content_and_has_no_bogus_target() {
        // The real sensor stamps the mouse position on key events too; sample_wire keeps
        // x/y set to mirror that. A keystroke must still record no content and no target.
        let wire = sample_wire("human.mac.key");
        let event = mac_wire_to_event(3, wire);
        assert_eq!(event.seq, 3);
        let human = event.human.expect("human payload");
        // The keystroke is recorded as an occurrence; its content is omitted by policy.
        match human.value {
            Some(HumanValue::Omitted { .. }) => {}
            other => panic!("expected omitted key value, got {other:?}"),
        }
        // No AX role resolved for a key, so the unrelated mouse coordinate must NOT become
        // a spurious target.
        assert!(
            human.target.is_none(),
            "a keystroke must not carry a coordinate target"
        );
    }

    #[test]
    fn wire_event_ndjson_roundtrips() {
        let mut wire = sample_wire("human.mac.scroll");
        wire.scroll_delta = Some(-3);
        wire.role = Some("AXScrollArea".into());
        let line = serde_json::to_string(&wire).unwrap();
        let back: MacWireEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(wire, back);
    }
}
