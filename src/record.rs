//! Recording control: active-session flag, span open/close, and metadata on stop.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ulid::Ulid;

use crate::{catalog, paths, span};

/// State of the active recording, serialized in `~/.galdr/active`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveRec {
    pub rec_id: String,
    pub name: String,
    pub started_at: String,
    /// Session transcript path; the sensor captures it from the first event.
    #[serde(default)]
    pub transcript_path: Option<String>,
}

/// Metadata of a closed recording, serialized in
/// `~/.galdr/recordings/<rec_id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub rec_id: String,
    pub name: String,
    pub started_at: String,
    pub ended_at: String,
    pub steps: usize,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Current timestamp in RFC3339 (UTC).
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// Reads the active-recording flag. `None` if there is none (or if the flag is
/// corrupt: treated as "not recording", which is the safe side).
pub fn read_active() -> Option<ActiveRec> {
    let path = paths::active_flag().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Writes (or overwrites) the active-recording flag.
pub fn write_active(active: &ActiveRec) -> Result<()> {
    paths::ensure_dirs()?;
    let path = paths::active_flag()?;
    std::fs::write(path, serde_json::to_string_pretty(active)?)?;
    Ok(())
}

/// Starts a recording. Fails if one is already active.
pub fn start(name: Option<String>) -> Result<()> {
    if let Some(existing) = read_active() {
        bail!(
            "a recording is already active: {} ({}). Run `galdr rec stop` first.",
            existing.name,
            existing.rec_id
        );
    }
    paths::ensure_dirs()?;

    let rec_id = Ulid::new().to_string();
    let name = name.unwrap_or_else(|| "rec".to_string());
    let active = ActiveRec {
        rec_id: rec_id.clone(),
        name: name.clone(),
        started_at: now_rfc3339(),
        transcript_path: None,
    };
    write_active(&active)?;

    // Open an empty span for this recording (touch, without truncating anything).
    let span_path = paths::span_file(&rec_id)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&span_path)
        .with_context(|| format!("could not open span {}", span_path.display()))?;

    println!("● recording \"{name}\"  rec_id={rec_id}");
    println!("  span: {}", span_path.display());
    println!("  stop with: galdr rec stop");
    Ok(())
}

/// Stops the active recording and writes its metadata.
pub fn stop() -> Result<()> {
    let active = read_active().context("no active recording")?;
    let span_path = paths::span_file(&active.rec_id)?;
    let events = span::read_span(&span_path).unwrap_or_default();
    let steps = events.len();
    let cwd = events.last().and_then(|e| e.cwd.clone());

    let recording = Recording {
        rec_id: active.rec_id.clone(),
        name: active.name.clone(),
        started_at: active.started_at.clone(),
        ended_at: now_rfc3339(),
        steps,
        cwd,
    };
    paths::ensure_dirs()?;
    let rec_path = paths::recording_file(&active.rec_id)?;
    std::fs::write(&rec_path, serde_json::to_string_pretty(&recording)?)?;

    // Drop the flag: from here on the sensor stops recording.
    let _ = std::fs::remove_file(paths::active_flag()?);

    // Keep the local catalog current even when the daemon is not running. This
    // is best-effort because the span + recording metadata are the truth.
    let _ = catalog::sync_closed_recording(&recording, &events);

    // Best-effort, after the metadata is on disk: tell the daemon to index it.
    crate::ipc::notify_best_effort(&crate::ipc::Request::RecordingClosed {
        recording: recording.clone(),
    });

    println!(
        "■ recording stopped \"{}\"  rec_id={}",
        active.name, active.rec_id
    );
    println!("  steps: {steps}");
    println!("  distill with: galdr distill {}", active.rec_id);
    Ok(())
}

/// Lists closed recordings, newest first (the rec_id is a ULID, time-sortable).
pub fn list() -> Result<()> {
    let dir = paths::recordings_dir()?;
    let mut recordings: Vec<Recording> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(contents) = std::fs::read_to_string(&path)
                && let Ok(rec) = serde_json::from_str::<Recording>(&contents)
            {
                recordings.push(rec);
            }
        }
    }
    recordings.sort_by(|a, b| b.rec_id.cmp(&a.rec_id));

    if recordings.is_empty() {
        println!("(no recordings yet — use `galdr rec start <name>`)");
        return Ok(());
    }
    for rec in &recordings {
        println!(
            "{}  {:<20}  {} steps  {}",
            rec.rec_id, rec.name, rec.steps, rec.started_at
        );
    }
    Ok(())
}
