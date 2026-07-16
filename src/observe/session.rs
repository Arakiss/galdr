#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ObserveFixture {
    BrowserForm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserObserveSession {
    rec_id: String,
    name: String,
    url: String,
    started_at: String,
    port: u16,
    devtools_port: u16,
    session_dir: PathBuf,
    sensor_dir: PathBuf,
    profile_dir: PathBuf,
    events_file: PathBuf,
    log_file: PathBuf,
    cwd: Option<String>,
    #[serde(default)]
    server_pid: Option<u32>,
    #[serde(default)]
    browser_pid: Option<u32>,
    #[serde(default)]
    headless: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrowserWireEvent {
    ts: String,
    action: String,
    source: HumanSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<HumanTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<HumanValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verification_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    frame_ref: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CdpTarget {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

pub fn synthetic(name: String, fixture: ObserveFixture) -> Result<()> {
    paths::ensure_dirs()?;

    let rec_id = Ulid::new().to_string();
    let started_at = record::now_rfc3339();
    let events = match fixture {
        ObserveFixture::BrowserForm => browser_form_events(&started_at),
    };
    let recording = record::Recording {
        rec_id: rec_id.clone(),
        name,
        started_at,
        ended_at: record::now_rfc3339(),
        steps: events.len(),
        cwd: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
        closed_reason: None,
    };

    write_recording_files(&recording, &events)?;
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
        "{} observed \"{}\" — {} human steps",
        style::accent("◆"),
        recording.name,
        events.len()
    );
    println!("  rec_id: {}", recording.rec_id);
    println!(
        "  fixture: {}",
        match fixture {
            ObserveFixture::BrowserForm => "browser-form",
        }
    );
    println!(
        "  turn it into a skill:  galdr distill {}",
        recording.rec_id
    );
    Ok(())
}

pub fn browser_start(
    name: String,
    url: String,
    browser: Option<PathBuf>,
    no_open: bool,
    headless: bool,
) -> Result<()> {
    paths::ensure_dirs()?;
    if paths::browser_observe_active()?.exists() {
        bail!("a browser observation is already active. Run `galdr observe browser stop` first.");
    }

    let rec_id = Ulid::new().to_string();
    let started_at = record::now_rfc3339();
    let port = reserve_loopback_port()?;
    let devtools_port = reserve_loopback_port()?;
    let session_dir = paths::browser_observe_session_dir(&rec_id)?;
    let sensor_dir = session_dir.join("sensor");
    let profile_dir = session_dir.join("profile");
    let events_file = session_dir.join("events.ndjson");
    let log_file = session_dir.join("server.log");
    std::fs::create_dir_all(&sensor_dir)?;
    std::fs::create_dir_all(&profile_dir)?;
    std::fs::write(&events_file, "")?;

    let mut session = BrowserObserveSession {
        rec_id,
        name,
        url,
        started_at,
        port,
        devtools_port,
        session_dir,
        sensor_dir,
        profile_dir,
        events_file,
        log_file,
        cwd: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
        server_pid: None,
        browser_pid: None,
        headless,
    };
    write_browser_sensor_artifacts(&session)?;
    write_session(&session)?;
    write_active_session(&session)?;

    let server_pid = spawn_browser_server(&session)?;
    session.server_pid = Some(server_pid);
    write_session(&session)?;
    write_active_session(&session)?;
    wait_for_server(session.port)?;

    if !no_open {
        let browser_path = browser.or_else(find_browser).context(
            "no Chrome/Chromium-compatible browser found; pass --browser or use --no-open",
        )?;
        let browser_pid = launch_browser(&browser_path, &session)?;
        session.browser_pid = Some(browser_pid);
        write_session(&session)?;
        write_active_session(&session)?;
        if let Err(err) = install_cdp_observer(&session) {
            let _ = post_shutdown(session.port);
            terminate_process(session.browser_pid);
            terminate_browser_profile(&session.profile_dir);
            let _ = std::fs::remove_file(paths::browser_observe_active()?);
            return Err(err).context("could not install the browser observe CDP sensor");
        }
    }

    println!(
        "{} observing browser \"{}\"",
        style::accent("◆"),
        session.name
    );
    println!("  rec_id: {}", session.rec_id);
    println!("  url: {}", session.url);
    println!("  loopback: http://127.0.0.1:{}/event", session.port);
    println!(
        "  devtools: http://127.0.0.1:{}/json/list",
        session.devtools_port
    );
    if no_open {
        println!("  browser: not launched (--no-open)");
    } else if headless {
        println!("  browser: launched headless with a local CDP observer");
    } else {
        println!("  browser: launched with a local CDP observer");
    }
    println!("  stop:  galdr observe browser stop");
    Ok(())
}

pub fn browser_status() -> Result<()> {
    let Some(session) = read_active_session()? else {
        println!("no active browser observation");
        return Ok(());
    };
    let event_count = count_event_lines(&session.events_file);
    let live = server_healthy(session.port);
    println!("active browser observation: {}", session.name);
    println!("  rec_id: {}", session.rec_id);
    println!("  url: {}", session.url);
    println!("  events: {event_count}");
    println!("  server: {}", if live { "up" } else { "not answering" });
    println!(
        "  devtools: http://127.0.0.1:{}/json/list",
        session.devtools_port
    );
    if let Some(pid) = session.server_pid {
        println!("  server_pid: {pid}");
    }
    if let Some(pid) = session.browser_pid {
        println!("  browser_pid: {pid}");
    }
    Ok(())
}

pub fn browser_stop() -> Result<()> {
    let Some(session) = read_active_session()? else {
        println!("no active browser observation");
        return Ok(());
    };
    let _ = post_shutdown(session.port);
    terminate_process(session.browser_pid);
    terminate_browser_profile(&session.profile_dir);
    let wire_events = read_browser_events(&session.events_file)?;
    let events: Vec<Event> = wire_events
        .into_iter()
        .enumerate()
        .map(|(idx, event)| browser_wire_to_event(idx as u64, event))
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
    let _ = std::fs::remove_file(paths::browser_observe_active()?);
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
        "{} stopped browser observation \"{}\" — {} human steps",
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

pub fn browser_serve(rec_id: &str) -> Result<()> {
    let session = read_session(rec_id)?;
    let listener = TcpListener::bind(("127.0.0.1", session.port))
        .with_context(|| format!("could not bind browser observe port {}", session.port))?;
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        if handle_browser_http(stream, &session)? {
            break;
        }
    }
    Ok(())
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
