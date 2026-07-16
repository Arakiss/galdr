//! Human-observation recording lanes.
//!
//! `rec` records agent tool calls through harness hooks. `observe` is the second
//! sensor lane: human-demonstrated actions written as typed span events. The
//! synthetic fixture exercises storage and distillation deterministically; the
//! browser sensor uses loopback CDP to inject the local event collector into an
//! isolated Chrome profile.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use ulid::Ulid;

use crate::span::{
    Event, EventKind, HumanAction, HumanEvent, HumanSource, HumanTarget, HumanValue, TargetLocator,
};
use crate::{catalog, engine, ipc, paths, record, span, style};

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

fn reserve_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn session_file(rec_id: &str) -> Result<PathBuf> {
    Ok(paths::browser_observe_session_dir(rec_id)?.join("session.json"))
}

fn write_session(session: &BrowserObserveSession) -> Result<()> {
    std::fs::create_dir_all(&session.session_dir)?;
    std::fs::write(
        session_file(&session.rec_id)?,
        serde_json::to_string_pretty(session)?,
    )?;
    Ok(())
}

fn read_session(rec_id: &str) -> Result<BrowserObserveSession> {
    let path = session_file(rec_id)?;
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_active_session(session: &BrowserObserveSession) -> Result<()> {
    std::fs::write(
        paths::browser_observe_active()?,
        serde_json::to_string_pretty(session)?,
    )?;
    Ok(())
}

fn read_active_session() -> Result<Option<BrowserObserveSession>> {
    let path = paths::browser_observe_active()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&raw)?))
}

fn spawn_browser_server(session: &BrowserObserveSession) -> Result<u32> {
    let exe = std::env::current_exe().context("could not find the galdr executable")?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session.log_file)?;
    let err = log.try_clone()?;
    let child = std::process::Command::new(exe)
        .args(["observe", "browser", "serve", &session.rec_id])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .context("could not spawn browser observe server")?;
    Ok(child.id())
}

fn wait_for_server(port: u16) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if server_healthy(port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("browser observe server did not answer on 127.0.0.1:{port}");
}

fn server_healthy(port: u16) -> bool {
    request_loopback(port, "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .map(|resp| resp.starts_with("HTTP/1.1 200"))
        .unwrap_or(false)
}

fn post_shutdown(port: u16) -> Result<()> {
    request_loopback(
        port,
        "POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n",
    )
    .map(|_| ())
}

fn request_loopback(port: u16, request: &str) -> Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(request.as_bytes())?;
    let mut bytes = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
            Err(err)
                if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                    && !bytes.is_empty() =>
            {
                break;
            }
            Err(err) => return Err(err.into()),
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn install_cdp_observer(session: &BrowserObserveSession) -> Result<()> {
    let target = wait_for_cdp_target(session)?;
    let ws_url = target
        .web_socket_debugger_url
        .context("CDP page target did not expose a websocket URL")?;
    validate_cdp_ws_loopback(&ws_url)?;
    let (mut socket, _) = tungstenite::connect(ws_url.as_str())
        .with_context(|| format!("could not connect to CDP websocket {ws_url}"))?;
    let script = browser_content_script(session.port);
    let mut next_id = 1u64;
    cdp_call(&mut socket, next_id, "Page.enable", serde_json::json!({}))?;
    next_id += 1;
    cdp_call(
        &mut socket,
        next_id,
        "Runtime.enable",
        serde_json::json!({}),
    )?;
    next_id += 1;
    cdp_call(
        &mut socket,
        next_id,
        "Page.addScriptToEvaluateOnNewDocument",
        serde_json::json!({ "source": script }),
    )?;
    next_id += 1;
    cdp_call(
        &mut socket,
        next_id,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": browser_content_script(session.port),
            "awaitPromise": false,
            "returnByValue": false
        }),
    )?;
    Ok(())
}

fn wait_for_cdp_target(session: &BrowserObserveSession) -> Result<CdpTarget> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_error = None;
    let mut fallback = None;
    while Instant::now() < deadline {
        match cdp_targets(session.devtools_port) {
            Ok(targets) => {
                if let Some(target) = targets
                    .iter()
                    .find(|target| target.kind == "page" && target.url == session.url)
                    .cloned()
                {
                    return Ok(target);
                }
                fallback = targets
                    .iter()
                    .find(|target| {
                        target.kind == "page" && target.web_socket_debugger_url.is_some()
                    })
                    .cloned()
                    .or(fallback);
            }
            Err(err) => last_error = Some(err),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(target) = fallback {
        return Ok(target);
    }
    if let Some(err) = last_error {
        return Err(err).context("CDP target discovery failed");
    }
    bail!(
        "no page target found on Chrome DevTools at 127.0.0.1:{}",
        session.devtools_port
    )
}

fn cdp_targets(port: u16) -> Result<Vec<CdpTarget>> {
    let request =
        format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    let response = request_loopback(port, &request)?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or(response.as_str());
    Ok(serde_json::from_str(body)?)
}

fn validate_cdp_ws_loopback(ws_url: &str) -> Result<()> {
    let http_url = ws_url
        .strip_prefix("ws://")
        .map(|rest| format!("http://{rest}"))
        .context("CDP websocket URL must use ws:// loopback")?;
    engine::validate_loopback(&http_url)?;
    Ok(())
}

fn cdp_call(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let request = serde_json::json!({
        "id": id,
        "method": method,
        "params": params
    });
    socket.send(Message::Text(request.to_string().into()))?;
    loop {
        let message = socket.read()?;
        let Message::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        if value.get("id").and_then(serde_json::Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            bail!("CDP call {method} failed: {error}");
        }
        if let Some(exception) = value
            .get("result")
            .and_then(|result| result.get("exceptionDetails"))
        {
            bail!("CDP call {method} raised an exception: {exception}");
        }
        return Ok(value);
    }
}

fn handle_browser_http(mut stream: TcpStream, session: &BrowserObserveSession) -> Result<bool> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_len = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut body)?;
    }
    let method_path = request_line.split_whitespace().take(2).collect::<Vec<_>>();
    let method = method_path.first().copied().unwrap_or("");
    let path = method_path.get(1).copied().unwrap_or("");
    match (method, path) {
        ("OPTIONS", _) => {
            write_http_response(&mut stream, 204, "No Content", "")?;
            Ok(false)
        }
        ("GET", "/health") => {
            write_http_response(&mut stream, 200, "OK", "ok")?;
            Ok(false)
        }
        ("POST", "/event") => {
            let body = String::from_utf8_lossy(&body);
            if serde_json::from_str::<BrowserWireEvent>(&body).is_ok() {
                append_browser_event(&session.events_file, &body)?;
            }
            write_http_response(&mut stream, 204, "No Content", "")?;
            Ok(false)
        }
        ("POST", "/shutdown") => {
            write_http_response(&mut stream, 200, "OK", "bye")?;
            Ok(true)
        }
        _ => {
            write_http_response(&mut stream, 404, "Not Found", "not found")?;
            Ok(false)
        }
    }
}

fn write_http_response(stream: &mut TcpStream, code: u16, reason: &str, body: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Allow-Methods: GET,POST,OPTIONS\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )?;
    Ok(())
}

fn append_browser_event(path: &Path, body: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(body.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn count_event_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|raw| raw.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

fn read_browser_events(path: &Path) -> Result<Vec<BrowserWireEvent>> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let mut events = Vec::new();
    for line in raw.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Ok(event) = serde_json::from_str::<BrowserWireEvent>(line) {
            events.push(event);
        }
    }
    Ok(events)
}

fn browser_wire_to_event(seq: u64, event: BrowserWireEvent) -> Event {
    let action = event.action;
    Event {
        ts: event.ts,
        seq,
        tool_name: action.clone(),
        tool_input: serde_json::Value::Null,
        tool_response: serde_json::Value::Null,
        cwd: None,
        session_id: None,
        event_kind: EventKind::Human,
        human: Some(HumanEvent {
            source: event.source,
            action: HumanAction(action),
            target: event.target,
            value: event.value,
            verification_hint: event.verification_hint,
            frame_ref: event.frame_ref,
        }),
    }
}

fn write_browser_sensor_artifacts(session: &BrowserObserveSession) -> Result<()> {
    std::fs::create_dir_all(&session.sensor_dir)?;
    std::fs::write(
        session.sensor_dir.join("content.js"),
        browser_content_script(session.port),
    )?;
    Ok(())
}

fn browser_content_script(port: u16) -> String {
    format!(
        r##"(function () {{
  if (window.__galdrObserveInstalled) return;
  window.__galdrObserveInstalled = true;

  const endpoint = "http://127.0.0.1:{port}/event";
  const sentNavigate = new Set();

  function source() {{
    return {{
      kind: "browser",
      url: window.location.href,
      title: document.title || null
    }};
  }}

  function textOf(el) {{
    if (!el) return null;
    const tag = el.tagName ? el.tagName.toLowerCase() : "";
    if (tag === "input" || tag === "textarea" || tag === "select") return null;
    const value = (el.innerText || el.textContent || "").trim();
    return value ? value.slice(0, 120) : null;
  }}

  function cssFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return "#" + CSS.escape(el.id);
    const testId = el.getAttribute("data-testid") || el.getAttribute("data-test");
    if (testId) return "[data-testid=\"" + CSS.escape(testId) + "\"]";
    const tag = el.tagName.toLowerCase();
    const name = el.getAttribute("name");
    if (name) return tag + "[name=\"" + CSS.escape(name) + "\"]";
    return tag;
  }}

  function targetFor(node) {{
    const el = node && node.closest ? node.closest("button,a,input,textarea,select,label,[role],[data-testid],[data-test]") : node;
    if (!el || !el.getAttribute) return null;
    const role = el.getAttribute("role") || (el.tagName ? el.tagName.toLowerCase() : null);
    const aria = el.getAttribute("aria-label");
    const label = labelsFor(el);
    const placeholder = el.getAttribute("placeholder");
    const text = textOf(el);
    const css = cssFor(el);
    const primary = label
      ? {{ kind: "label", value: label }}
      : placeholder
        ? {{ kind: "placeholder", value: placeholder }}
        : aria
          ? {{ kind: "role", role: role || "element", name: aria }}
          : css
            ? {{ kind: "css", value: css }}
            : {{ kind: "css", value: "element" }};
    return {{
      primary,
      role,
      name: aria || label || text || placeholder || null,
      text,
      label,
      placeholder,
      element_summary: el.tagName ? el.tagName.toLowerCase() : null
    }};
  }}

  function labelsFor(el) {{
    if (!el) return null;
    if (el.labels && el.labels.length) {{
      const label = Array.from(el.labels).map(l => (l.innerText || l.textContent || "").trim()).find(Boolean);
      if (label) return label.slice(0, 120);
    }}
    const id = el.getAttribute && el.getAttribute("id");
    if (id) {{
      const label = document.querySelector("label[for='" + CSS.escape(id) + "']");
      if (label) {{
        const text = (label.innerText || label.textContent || "").trim();
        if (text) return text.slice(0, 120);
      }}
    }}
    const closest = el.closest && el.closest("label");
    if (closest) {{
      const text = (closest.innerText || closest.textContent || "").trim();
      if (text) return text.slice(0, 120);
    }}
    return null;
  }}

  function emit(action, target, value, verificationHint) {{
    const event = {{
      ts: new Date().toISOString(),
      action,
      source: source(),
      target: target || null,
      value: value || null,
      verification_hint: verificationHint || null
    }};
    const body = JSON.stringify(event);
    fetch(endpoint, {{
      method: "POST",
      body,
      keepalive: true
    }}).catch(() => {{
      if (navigator.sendBeacon) {{
        try {{ navigator.sendBeacon(endpoint, body); }} catch (_) {{}}
      }}
    }});
  }}

  function emitNavigate() {{
    const key = window.location.href + "::" + document.title;
    if (sentNavigate.has(key)) return;
    sentNavigate.add(key);
    emit("human.browser.navigate", null, null, null);
  }}

  function inputValueFor(el) {{
    const type = (el.getAttribute("type") || "").toLowerCase();
    const name = [el.getAttribute("name"), el.getAttribute("id"), labelsFor(el), el.getAttribute("placeholder")]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    if (type === "password") {{
      return {{ policy: "omitted", reason: "password-field" }};
    }}
    if (/password|passwd|token|secret|api[_-]?key|authorization|credential|private[_-]?key/.test(name)) {{
      return {{ policy: "redacted", kind: "sensitive-field", chars: (el.value || "").length }};
    }}
    return {{ policy: "redacted", kind: "text", chars: (el.value || "").length }};
  }}

  document.addEventListener("DOMContentLoaded", emitNavigate, true);
  window.addEventListener("load", emitNavigate, true);
  setTimeout(emitNavigate, 0);
  setTimeout(emitNavigate, 500);
  document.addEventListener("click", event => {{
    emit("human.browser.click", targetFor(event.target), null, null);
  }}, true);
  document.addEventListener("input", event => {{
    const el = event.target;
    if (!el || !("value" in el)) return;
    emit("human.browser.input", targetFor(el), inputValueFor(el), null);
  }}, true);
  document.addEventListener("change", event => {{
    const el = event.target;
    if (!el || !el.tagName) return;
    const tag = el.tagName.toLowerCase();
    if (tag === "select") {{
      const selected = el.options && el.selectedIndex >= 0 ? el.options[el.selectedIndex].text : el.value;
      emit("human.browser.select", targetFor(el), {{ policy: "literal", value: selected || el.value || "" }}, null);
    }} else if (el.type === "checkbox" || el.type === "radio") {{
      emit("human.browser.check", targetFor(el), {{ policy: "literal", value: el.checked ? "true" : "false" }}, null);
    }}
  }}, true);
  document.addEventListener("keydown", event => {{
    const printable = event.key && event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey;
    if (printable) return;
    const combo = [
      event.metaKey ? "Meta" : null,
      event.ctrlKey ? "Ctrl" : null,
      event.altKey ? "Alt" : null,
      event.shiftKey ? "Shift" : null,
      event.key
    ].filter(Boolean).join("+");
    emit("human.browser.key", targetFor(event.target), {{ policy: "literal", value: combo }}, null);
  }}, true);
}})();
"##
    )
}

fn find_browser() -> Option<PathBuf> {
    for path in [
        "/Applications/Google Chrome.app",
        "/Applications/Chromium.app",
        "/Applications/Brave Browser.app",
        "/Applications/Microsoft Edge.app",
    ] {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    for name in [
        "google-chrome",
        "chromium",
        "chromium-browser",
        "brave-browser",
        "microsoft-edge",
    ] {
        if let Some(path) = find_on_path(name) {
            return Some(path);
        }
    }
    None
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn launch_browser(browser: &Path, session: &BrowserObserveSession) -> Result<u32> {
    let args = browser_args(session);
    let browser_path = browser_executable(browser);
    let child = std::process::Command::new(&browser_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not launch browser {}", browser_path.display()))?;
    Ok(child.id())
}

fn browser_args(session: &BrowserObserveSession) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", session.profile_dir.display()),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--remote-debugging-port={}", session.devtools_port),
        "--remote-allow-origins=*".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-sync".to_string(),
        "--disable-features=PasswordManagerOnboarding".to_string(),
        "--password-store=basic".to_string(),
        "--use-mock-keychain".to_string(),
    ];
    if session.headless {
        args.push("--headless=new".to_string());
        args.push("--disable-gpu".to_string());
    } else {
        args.push("--new-window".to_string());
    }
    args.push(session.url.clone());
    args
}

fn is_macos_app(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("app")
}

fn browser_executable(browser: &Path) -> PathBuf {
    if !is_macos_app(browser) {
        return browser.to_path_buf();
    }
    let Some(name) = browser.file_stem().and_then(|name| name.to_str()) else {
        return browser.to_path_buf();
    };
    let executable = browser.join("Contents").join("MacOS").join(name);
    if executable.exists() {
        executable
    } else {
        browser.to_path_buf()
    }
}

fn terminate_process(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
}

fn terminate_browser_profile(profile_dir: &Path) {
    #[cfg(unix)]
    {
        let marker = format!("--user-data-dir={}", profile_dir.display());
        let Ok(output) = std::process::Command::new("ps")
            .args(["-axo", "pid=,command="])
            .output()
        else {
            return;
        };
        let raw = String::from_utf8_lossy(&output.stdout);
        for line in raw.lines() {
            if !line.contains(&marker) {
                continue;
            }
            let Some(pid) = line
                .split_whitespace()
                .next()
                .and_then(|raw| raw.parse::<u32>().ok())
            else {
                continue;
            };
            terminate_process(Some(pid));
        }
    }
    #[cfg(not(unix))]
    let _ = profile_dir;
}

fn browser_form_events(ts: &str) -> Vec<Event> {
    vec![
        human_event(
            ts,
            0,
            "human.browser.navigate",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.navigate"),
                target: None,
                value: None,
                verification_hint: None,
                frame_ref: None,
            },
        ),
        human_event(
            ts,
            1,
            "human.browser.input",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.input"),
                target: Some(label_target("Issue title")),
                value: Some(HumanValue::Redacted {
                    kind: "text".to_string(),
                    chars: Some(24),
                }),
                verification_hint: None,
                frame_ref: None,
            },
        ),
        human_event(
            ts,
            2,
            "human.browser.select",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.select"),
                target: Some(label_target("Priority")),
                value: Some(HumanValue::Literal {
                    value: "High".to_string(),
                }),
                verification_hint: None,
                frame_ref: None,
            },
        ),
        human_event(
            ts,
            3,
            "human.browser.click",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.click"),
                target: Some(role_target("button", "Create issue")),
                value: None,
                verification_hint: Some(
                    "Confirm the created issue page is open or a success message appears."
                        .to_string(),
                ),
                frame_ref: None,
            },
        ),
    ]
}

fn human_event(ts: &str, seq: u64, tool_name: &str, human: HumanEvent) -> Event {
    Event {
        ts: ts.to_string(),
        seq,
        tool_name: tool_name.to_string(),
        tool_input: serde_json::Value::Null,
        tool_response: serde_json::Value::Null,
        cwd: None,
        session_id: None,
        event_kind: EventKind::Human,
        human: Some(human),
    }
}

fn browser_source(url: &str, title: &str) -> HumanSource {
    HumanSource::Browser {
        url: Some(url.to_string()),
        title: Some(title.to_string()),
        tab_id: None,
    }
}

fn label_target(label: &str) -> HumanTarget {
    HumanTarget {
        primary: TargetLocator::Label {
            value: label.to_string(),
        },
        alternates: Vec::new(),
        role: None,
        name: None,
        text: None,
        label: Some(label.to_string()),
        placeholder: None,
        element_summary: None,
    }
}

fn role_target(role: &str, name: &str) -> HumanTarget {
    HumanTarget {
        primary: TargetLocator::Role {
            role: role.to_string(),
            name: Some(name.to_string()),
        },
        alternates: Vec::new(),
        role: Some(role.to_string()),
        name: Some(name.to_string()),
        text: None,
        label: None,
        placeholder: None,
        element_summary: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_launch_args_avoid_keychain_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let session = BrowserObserveSession {
            rec_id: "rec".to_string(),
            name: "demo".to_string(),
            url: "https://example.test".to_string(),
            started_at: "2026-06-30T00:00:00Z".to_string(),
            port: 34343,
            devtools_port: 34344,
            session_dir: tmp.path().to_path_buf(),
            sensor_dir: tmp.path().join("sensor"),
            profile_dir: tmp.path().join("profile"),
            events_file: tmp.path().join("events.ndjson"),
            log_file: tmp.path().join("server.log"),
            cwd: None,
            server_pid: None,
            browser_pid: None,
            headless: false,
        };

        let args = browser_args(&session);

        assert!(args.iter().any(|arg| arg == "--use-mock-keychain"));
        assert!(args.iter().any(|arg| arg == "--password-store=basic"));
        assert!(args.iter().any(|arg| arg == "--disable-sync"));
        assert!(
            args.iter()
                .any(|arg| arg == "--remote-debugging-address=127.0.0.1")
        );
        assert!(
            args.iter()
                .any(|arg| arg == "--remote-debugging-port=34344")
        );
        assert!(!args.iter().any(|arg| arg.starts_with("--load-extension")));
        assert!(
            args.iter().any(|arg| {
                arg == &format!("--user-data-dir={}", session.profile_dir.display())
            })
        );
    }

    #[test]
    fn browser_executable_resolves_macos_app_bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Test Browser.app");
        let executable = app.join("Contents/MacOS/Test Browser");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, "").unwrap();

        assert_eq!(browser_executable(&app), executable);
    }
}
