fn cmd_rec_status() -> anyhow::Result<()> {
    let actives = record::read_active_all();
    if actives.is_empty() {
        println!("no active recording");
        return Ok(());
    }
    let n = actives.len();
    let stale_after_hours = config::Config::load_capture().stale_after_hours;
    println!("{n} active recording{}:", if n == 1 { "" } else { "s" });
    for active in &actives {
        let span_path = paths::span_file(&active.rec_id)?;
        let steps = span::count_events(&span_path);
        println!("  {} ({})", active.name, active.rec_id);
        if record::is_stale(active, stale_after_hours) {
            let hours = record::inactive_hours(active).unwrap_or_default();
            println!(
                "    {}",
                style::amber(&format!(
                    "STALE — inactive for {hours}h; it no longer captures new sessions. Close it: galdr rec stop {}",
                    active.name
                ))
            );
        }
        println!("    started_at: {}", active.started_at);
        println!("    steps: {steps}");
        if let Some(origin) = &active.origin_cwd {
            println!("    origin_cwd: {origin}");
        }
        match &active.bound_session {
            Some(session) => println!("    bound_session: {session}"),
            None => println!(
                "    bound_session: (unbound — waiting for the first activity of a session that has no recording yet)"
            ),
        }
        println!("    span: {}", span_path.display());
        if let Some(transcript_path) = &active.transcript_path {
            println!("    transcript: {transcript_path}");
        }
    }
    Ok(())
}

fn cmd_daemon_status() -> anyhow::Result<()> {
    match ipc::query(&ipc::Request::Ping) {
        Ok(ipc::Response::Pong { version }) => {
            println!("daemon running");
            match version.as_deref() {
                Some(v) => println!("  version: {v}"),
                None => println!("  version: unknown (older daemon; restart to report it)"),
            }
            if let Ok(pid) = std::fs::read_to_string(paths::pidfile()?) {
                println!("  pid: {}", pid.trim());
            }
        }
        _ => {
            let pidfile = paths::pidfile()?;
            if pidfile.exists() {
                println!(
                    "daemon not responding (stale pidfile: {})",
                    pidfile.display()
                );
            } else {
                println!("daemon stopped");
            }
        }
    }
    // Whether launchd manages it (macOS): managed, installed-but-not-loaded, or a loose
    // process. Omitted off macOS, where there is no launchd to report on.
    if let Some(line) = launchd::status_line() {
        println!("  {line}");
    }
    Ok(())
}

fn cmd_daemon_stop() -> anyhow::Result<()> {
    match ipc::query(&ipc::Request::Shutdown) {
        Ok(ipc::Response::Ack) => println!("daemon stopped"),
        Ok(other) => println!("daemon returned unexpected response: {other:?}"),
        Err(_) => println!("daemon is not running"),
    }
    Ok(())
}
