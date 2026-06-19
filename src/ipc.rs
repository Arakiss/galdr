//! Inter-process protocol between the sensor/CLI and the daemon.
//!
//! The wire format is NDJSON: one JSON object per line over a Unix-domain socket.
//! Both message families are internally tagged (`{"type": "...", ...}`), so the
//! protocol stays self-describing and forward-compatible — an unknown field is
//! ignored, a new request variant is just a new tag.
//!
//! The client here is deliberately **synchronous** and built on `std` only: the
//! sensor must never pull in the async runtime or depend on the daemon being up.
//! `notify_best_effort` is fire-and-forget with a tight timeout and swallows every
//! error; `query` is the CLI's request/response call with a short timeout.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::catalog::{RecordingDetail, RecordingRow, ReindexStats, SkillRow};
use crate::record::Recording;
use crate::span::Event;

/// How long the sensor's best-effort notify may spend before giving up. The
/// sensor's truth (the JSONL append) is already on disk; this is pure indexing
/// hint, so it must never add meaningful latency to the session.
const NOTIFY_TIMEOUT: Duration = Duration::from_millis(50);

/// How long the CLI waits on a daemon request before falling back to the DB.
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// A message from a client (sensor or CLI) to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Liveness probe.
    Ping,
    /// The sensor appended an event to a span (best-effort index hint).
    EventAppended { rec_id: String, event: Event },
    /// A recording was closed and its metadata written.
    RecordingClosed { recording: Recording },
    /// A skill was installed from a recording.
    SkillInstalled {
        skill_name: String,
        rec_id: String,
        skill_path: String,
    },
    /// List closed recordings.
    ListRecordings,
    /// Show one recording with its steps.
    ShowRecording { id: String },
    /// List installed skills with their provenance.
    ListSkills,
    /// Rebuild the catalog from disk.
    Reindex,
    /// Ask the daemon to shut down gracefully.
    Shutdown,
}

/// A message from the daemon back to a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Reply to `Ping`.
    Pong,
    /// Generic success with no payload (notifications, shutdown).
    Ack,
    /// Reply to `ListRecordings`.
    Recordings { recordings: Vec<RecordingRow> },
    /// Reply to `ShowRecording` (`None` if the id is unknown).
    Recording { recording: Option<RecordingDetail> },
    /// Reply to `ListSkills`.
    Skills { skills: Vec<SkillRow> },
    /// Reply to `Reindex`.
    Reindexed { stats: ReindexStats },
    /// Something went wrong handling the request.
    Error { message: String },
}

/// Fire-and-forget notification to the daemon. Connects, writes one line, and
/// returns; never waits for a reply, never blocks beyond [`NOTIFY_TIMEOUT`], and
/// swallows every error. If the daemon is down the sensor simply moves on — the
/// span on disk is already the source of truth, and a poll-watcher in the daemon
/// reconciles any missed events.
pub fn notify_best_effort(req: &Request) {
    let _ = send_notify(req);
}

fn send_notify(req: &Request) -> Result<()> {
    let path = crate::paths::socket_path()?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(NOTIFY_TIMEOUT))?;
    let mut line = serde_json::to_vec(req)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    Ok(())
}

/// Request/response call to the daemon for the CLI. Returns the parsed response,
/// or an error the caller uses to fall back to the read-only database.
pub fn query(req: &Request) -> Result<Response> {
    let path = crate::paths::socket_path()?;
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(QUERY_TIMEOUT))?;
    stream.set_write_timeout(Some(QUERY_TIMEOUT))?;

    let mut writer = stream.try_clone()?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;

    let mut reader = BufReader::new(stream);
    let mut resp_line = String::new();
    reader.read_line(&mut resp_line)?;
    let resp: Response = serde_json::from_str(resp_line.trim())?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_through_ndjson() {
        let req = Request::SkillInstalled {
            skill_name: "galdr-demo".into(),
            rec_id: "01ABC".into(),
            skill_path: "/x/SKILL.md".into(),
        };
        let line = serde_json::to_string(&req).unwrap();
        assert!(line.contains("\"type\":\"SkillInstalled\""));
        let back: Request = serde_json::from_str(&line).unwrap();
        match back {
            Request::SkillInstalled { skill_name, .. } => assert_eq!(skill_name, "galdr-demo"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn response_roundtrips_through_ndjson() {
        let resp = Response::Error {
            message: "nope".into(),
        };
        let line = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&line).unwrap();
        assert!(matches!(back, Response::Error { .. }));
    }

    #[test]
    fn notify_is_a_noop_without_a_daemon() {
        // No socket exists in the test environment: the call must not panic and
        // must return promptly (errors are swallowed).
        notify_best_effort(&Request::Ping);
    }
}
