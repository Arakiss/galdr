//! Supervisor daemon: a long-lived process that keeps the SQLite catalog in sync
//! with the spans on disk and answers queries over the control socket.
//!
//! Design constraints it honors:
//!
//! - **The sensor never depends on it.** Notifications are best-effort; if the
//!   daemon is down, nothing breaks — a poll-watcher reconciles missed events when
//!   it comes back up.
//! - **Single instance.** Startup probes the socket: a live daemon answers `Pong`
//!   and we bow out; a stale socket (connection refused / absent) is unlinked.
//! - **Self-healing index.** It reindexes from disk on startup, so a corrupt or
//!   missing catalog is repaired automatically.
//! - **No network.** It binds a Unix-domain socket under `~/.galdr`, chmod 0600.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::Notify;
use tokio::time::interval;

use crate::ipc::{Request, Response};
use crate::{catalog, ipc, paths, record};

type Db = Arc<Mutex<Connection>>;

/// Entry point for `galdr daemon`. With `detach`, it re-execs itself in the
/// background and returns; otherwise it runs the event loop until a shutdown
/// signal or request.
pub fn run(detach: bool) -> Result<()> {
    // Fail fast, with an actionable message, before a long socket path turns into a
    // cryptic bind() error inside a detached child whose stderr is gone.
    validate_socket_path()?;
    if detach {
        return spawn_detached();
    }
    if already_running() {
        println!("galdr daemon already running");
        return Ok(());
    }
    // A leftover socket from a crashed daemon would block bind(); the probe above
    // proved no one is listening, so remove it.
    let sock = paths::socket_path()?;
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }
    paths::ensure_dirs()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("could not start the async runtime")?;
    rt.block_on(serve())
}

/// `Pong` over the socket means a live daemon owns it. Any error (connection
/// refused, missing socket) means it is free to take.
fn already_running() -> bool {
    matches!(ipc::query(&Request::Ping), Ok(Response::Pong))
}

/// Re-exec `galdr daemon` detached: own process group, null stdio. Dependency-free
/// (no libc); good enough for a CLI-launched background daemon.
///
/// The child's stderr is gone, so a startup failure would otherwise be silent and
/// `--detach` would happily report a pid for a process that already died. We poll
/// the control socket briefly and report the real outcome.
fn spawn_detached() -> Result<()> {
    let exe = std::env::current_exe().context("could not find the galdr executable")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let pid = cmd
        .spawn()
        .context("could not spawn the detached daemon")?
        .id();

    for _ in 0..40 {
        if already_running() {
            println!("galdr daemon started (pid {pid})");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!(
        "daemon spawned (pid {pid}) but never answered on the control socket within 2s; \
         it likely failed to start. Run `galdr daemon` in the foreground to see the error."
    );
}

/// Guards against the Unix-domain socket path exceeding `SUN_LEN` (104 bytes on
/// macOS, 108 on Linux), which would make `bind()` fail with an opaque error. A
/// long `GALDR_ROOT` or `$HOME` is the usual cause; the fix is a shorter root.
fn validate_socket_path() -> Result<()> {
    const MAX: usize = 100; // conservative across platforms, room for the NUL.
    let sock = paths::socket_path()?;
    let len = sock.as_os_str().as_bytes().len();
    if len > MAX {
        bail!(
            "daemon socket path is too long ({len} bytes, limit ~{MAX}): {}\n\
             Set GALDR_ROOT to a shorter directory (for example one under /tmp).",
            sock.display()
        );
    }
    Ok(())
}

async fn serve() -> Result<()> {
    let sock = paths::socket_path()?;
    let listener = UnixListener::bind(&sock)
        .with_context(|| format!("could not bind the control socket {}", sock.display()))?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    std::fs::write(paths::pidfile()?, std::process::id().to_string())?;

    // Heal a corrupt/missing catalog and rebuild it from disk before serving.
    let mut conn = open_or_heal()?;
    let stats = catalog::reindex(&mut conn)?;
    eprintln!(
        "galdr daemon: indexed {} recordings, {} steps, {} skills",
        stats.recordings, stats.steps, stats.skills
    );
    let db: Db = Arc::new(Mutex::new(conn));
    let shutdown = Arc::new(Notify::new());

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut poll = interval(Duration::from_secs(2));

    loop {
        tokio::select! {
            _ = shutdown.notified() => break,
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            _ = poll.tick() => {
                // Fill gaps from any best-effort notifications that never arrived.
                let db = db.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if let Ok(conn) = db.lock() {
                        let _ = catalog::reconcile(&conn);
                    }
                }).await;
            }
            accepted = listener.accept() => {
                if let Ok((stream, _addr)) = accepted {
                    tokio::spawn(handle_conn(stream, db.clone(), shutdown.clone()));
                }
            }
        }
    }

    // Graceful shutdown: checkpoint so a read-only fallback can open the WAL DB,
    // then drop the socket and pidfile.
    if let Ok(conn) = db.lock() {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    }
    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(paths::pidfile()?);
    Ok(())
}

/// Opens the catalog, healing it if the file is corrupt or unreadable.
fn open_or_heal() -> Result<Connection> {
    if let Ok(conn) = catalog::open() {
        return Ok(conn);
    }
    let db = paths::catalog_db()?;
    let _ = std::fs::remove_file(&db);
    let _ = std::fs::remove_file(db.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(db.with_extension("sqlite-shm"));
    catalog::open()
}

async fn handle_conn(stream: UnixStream, db: Db, shutdown: Arc<Notify>) {
    let _ = process_conn(stream, db, shutdown).await;
}

async fn process_conn(stream: UnixStream, db: Db, shutdown: Arc<Notify>) -> Result<()> {
    // Bound a request in size and time: a local client (the socket lives in the
    // 0700 root, so only this user, but still) must not be able to exhaust memory
    // with an endless line or hold a connection open forever without a newline.
    const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
    const READ_TIMEOUT: Duration = Duration::from_secs(10);
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    match tokio::time::timeout(READ_TIMEOUT, reader.read_line(&mut line)).await {
        Ok(Ok(0)) | Err(_) => return Ok(()),
        Ok(Ok(_)) => {}
        Ok(Err(err)) => return Err(err.into()),
    }

    let req: Request = match serde_json::from_str(line.trim()) {
        Ok(req) => req,
        Err(err) => {
            let resp = Response::Error {
                message: format!("bad request: {err}"),
            };
            let _ = write_response(&mut write_half, &resp).await;
            return Ok(());
        }
    };

    let is_shutdown = matches!(req, Request::Shutdown);
    let resp = handle_request(&db, req);
    // The client may already be gone (a best-effort notify never reads the reply);
    // a broken pipe here is expected and ignored.
    let _ = write_response(&mut write_half, &resp).await;
    if is_shutdown {
        shutdown.notify_one();
    }
    Ok(())
}

fn handle_request(db: &Db, req: Request) -> Response {
    match req {
        Request::Ping => Response::Pong,
        Request::Shutdown => Response::Ack,
        Request::EventAppended { rec_id, event } => with_db(db, |c| {
            catalog::index_event(c, &rec_id, &event).map(|()| Response::Ack)
        }),
        Request::RecordingClosed { recording } => with_db(db, |c| {
            catalog::index_recording(c, &recording).map(|()| Response::Ack)
        }),
        Request::SkillInstalled {
            skill_name,
            rec_id,
            skill_path,
            status,
        } => with_db(db, |c| {
            catalog::upsert_skill(
                c,
                &skill_name,
                Some(&rec_id),
                &skill_path,
                Some(&record::now_rfc3339()),
                &status,
            )
            .map(|()| Response::Ack)
        }),
        Request::ListRecordings => with_db(db, |c| {
            catalog::list_recordings(c).map(|recordings| Response::Recordings { recordings })
        }),
        Request::ShowRecording { id } => with_db(db, |c| {
            catalog::show_recording(c, &id).map(|recording| Response::Recording { recording })
        }),
        Request::ListSkills => with_db(db, |c| {
            catalog::list_skills(c).map(|skills| Response::Skills { skills })
        }),
        Request::Reindex => with_db_mut(db, |c| {
            catalog::reindex(c).map(|stats| Response::Reindexed { stats })
        }),
    }
}

fn with_db<F>(db: &Db, f: F) -> Response
where
    F: FnOnce(&Connection) -> Result<Response>,
{
    match db.lock() {
        Ok(conn) => f(&conn).unwrap_or_else(|err| Response::Error {
            message: err.to_string(),
        }),
        Err(_) => Response::Error {
            message: "catalog lock poisoned".into(),
        },
    }
}

fn with_db_mut<F>(db: &Db, f: F) -> Response
where
    F: FnOnce(&mut Connection) -> Result<Response>,
{
    match db.lock() {
        Ok(mut conn) => f(&mut conn).unwrap_or_else(|err| Response::Error {
            message: err.to_string(),
        }),
        Err(_) => Response::Error {
            message: "catalog lock poisoned".into(),
        },
    }
}

async fn write_response(w: &mut (impl AsyncWriteExt + Unpin), resp: &Response) -> Result<()> {
    let mut line = serde_json::to_vec(resp)?;
    line.push(b'\n');
    w.write_all(&line).await?;
    w.flush().await?;
    Ok(())
}
