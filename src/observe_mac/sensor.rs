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
