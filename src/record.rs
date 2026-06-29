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
    /// Directory where `rec start` ran. The sensor only binds the recording to a
    /// session whose first event happens under this tree, so a concurrent agent
    /// session in another project cannot leak its tool calls into this span.
    #[serde(default)]
    pub origin_cwd: Option<String>,
    /// The session id this recording locked onto, set by the sensor from the first
    /// event that matches `origin_cwd`. Once bound, only that session is recorded.
    #[serde(default)]
    pub bound_session: Option<String>,
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
    let origin_cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string());
    let active = ActiveRec {
        rec_id: rec_id.clone(),
        name: name.clone(),
        started_at: now_rfc3339(),
        transcript_path: None,
        origin_cwd,
        bound_session: None,
    };
    write_active(&active)?;

    // Open an empty span for this recording (touch, without truncating anything).
    let span_path = paths::span_file(&rec_id)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&span_path)
        .with_context(|| format!("could not open span {}", span_path.display()))?;

    println!("{} recording \"{name}\"", crate::style::red("●"));
    println!("  do the task, then:  galdr rec stop");
    Ok(())
}

/// Stops the active recording and writes its metadata.
pub fn stop() -> Result<()> {
    let active = read_active().context("no active recording")?;
    let span_path = paths::span_file(&active.rec_id)?;
    // Durably persist the span before we declare the recording closed. Best-effort:
    // a sync failure must not block stopping (the events are already in the file).
    let _ = span::fsync(&span_path);
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

    let plural = if steps == 1 { "" } else { "s" };
    println!(
        "{} stopped \"{}\" — {steps} step{plural}",
        crate::style::accent("■"),
        active.name
    );
    println!("  turn it into a skill:  galdr distill");
    Ok(())
}

/// All closed recordings, newest first (the rec_id is a ULID, time-sortable).
pub fn all_recordings() -> Vec<Recording> {
    let Ok(dir) = paths::recordings_dir() else {
        return Vec::new();
    };
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
    recordings
}

/// Resolves a human-friendly recording reference to a rec_id, so nobody has to copy a
/// 26-character ULID. `None` resolves to the **most recent** recording (so `galdr
/// distill` with no argument distills what you just recorded). A given reference matches,
/// in order: an exact rec_id, a unique rec_id prefix (case-insensitive), or a recording
/// **name** (the most recent of that name). Ambiguity and misses fail with a friendly,
/// actionable message rather than a cryptic id error.
pub fn resolve_ref(reference: Option<&str>) -> Result<String> {
    resolve_in(&all_recordings(), reference)
}

/// The pure matching behind [`resolve_ref`] (recordings newest-first). Separated so it
/// is unit tested without touching disk.
fn resolve_in(recordings: &[Recording], reference: Option<&str>) -> Result<String> {
    if recordings.is_empty() {
        bail!("no recordings yet — record one first with `galdr rec start <name>`.");
    }
    let Some(reference) = reference.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(recordings[0].rec_id.clone()); // newest
    };
    if let Some(rec) = recordings.iter().find(|r| r.rec_id == reference) {
        return Ok(rec.rec_id.clone());
    }
    let upper = reference.to_ascii_uppercase();
    let by_prefix: Vec<&Recording> = recordings
        .iter()
        .filter(|r| r.rec_id.starts_with(&upper))
        .collect();
    if by_prefix.len() == 1 {
        return Ok(by_prefix[0].rec_id.clone());
    }
    if by_prefix.len() > 1 {
        bail!(
            "`{reference}` matches {} recordings — add more characters, or use the name (see `galdr list`).",
            by_prefix.len()
        );
    }
    if let Some(rec) = recordings.iter().find(|r| r.name == reference) {
        return Ok(rec.rec_id.clone());
    }
    bail!("no recording matches `{reference}` — run `galdr list` to see your recordings.");
}

/// Lists closed recordings, newest first (the rec_id is a ULID, time-sortable).
pub fn list() -> Result<()> {
    let recordings = all_recordings();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, name: &str) -> Recording {
        Recording {
            rec_id: id.into(),
            name: name.into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            ended_at: "2026-01-01T00:01:00Z".into(),
            steps: 3,
            cwd: None,
        }
    }

    // Newest first, the way `all_recordings` returns them.
    fn fixture() -> Vec<Recording> {
        vec![
            rec("01KW9Z00000000000000000002", "weekly-report"),
            rec("01KW9Z00000000000000000001", "ship-preview"),
            rec("01KW9A00000000000000000000", "weekly-report"),
        ]
    }

    #[test]
    fn none_resolves_to_the_most_recent() {
        assert_eq!(
            resolve_in(&fixture(), None).unwrap(),
            "01KW9Z00000000000000000002"
        );
    }

    #[test]
    fn exact_id_and_unique_prefix_resolve() {
        let recs = fixture();
        assert_eq!(
            resolve_in(&recs, Some("01KW9Z00000000000000000001")).unwrap(),
            "01KW9Z00000000000000000001"
        );
        // A unique prefix is enough; matching is case-insensitive.
        assert_eq!(
            resolve_in(&recs, Some("01kw9z00000000000000000001")).unwrap(),
            "01KW9Z00000000000000000001"
        );
    }

    #[test]
    fn name_resolves_to_the_most_recent_of_that_name() {
        // Two "weekly-report" runs → the newest one wins.
        assert_eq!(
            resolve_in(&fixture(), Some("weekly-report")).unwrap(),
            "01KW9Z00000000000000000002"
        );
    }

    #[test]
    fn ambiguous_prefix_and_unknown_ref_fail_with_guidance() {
        let recs = fixture();
        // "01KW9Z" prefixes two recordings → ambiguous.
        let ambiguous = resolve_in(&recs, Some("01KW9Z")).unwrap_err().to_string();
        assert!(ambiguous.contains("matches 2 recordings"), "{ambiguous}");
        // A miss names the recovery command.
        let miss = resolve_in(&recs, Some("nope")).unwrap_err().to_string();
        assert!(miss.contains("galdr list"), "{miss}");
    }

    #[test]
    fn no_recordings_is_a_friendly_error() {
        let err = resolve_in(&[], None).unwrap_err().to_string();
        assert!(err.contains("galdr rec start"), "{err}");
    }
}
