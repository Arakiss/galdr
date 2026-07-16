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
