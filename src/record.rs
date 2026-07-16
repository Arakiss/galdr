//! Recording control: active-session flags, span open/close, and metadata on stop.

use std::path::Path;

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
    /// Why the recording closed, when it was not a plain `rec stop` (e.g. reaped as
    /// stale). Absent for ordinary stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_reason: Option<String>,
}

/// Current timestamp in RFC3339 (UTC).
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// Hours since this recording last saw activity: the span file's mtime when readable
/// (`rec start` creates the span and every append touches it), else the recording's
/// `started_at`. `None` when neither signal is available.
pub fn inactive_hours(active: &ActiveRec) -> Option<i64> {
    let span_mtime = paths::span_file(&active.rec_id)
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(OffsetDateTime::from);
    let last = span_mtime.or_else(|| OffsetDateTime::parse(&active.started_at, &Rfc3339).ok())?;
    Some((OffsetDateTime::now_utc() - last).whole_hours())
}

/// Whether an active recording has gone stale: no activity for at least
/// `stale_after_hours`. A forgotten `rec stop` used to leave a recording active
/// forever — swallowing session-less events and scaring every future session away
/// from recording. `0` disables staleness, and unknown activity is never stale:
/// a live recording must not be reaped on a guess.
pub fn is_stale(active: &ActiveRec, stale_after_hours: u64) -> bool {
    if stale_after_hours == 0 {
        return false;
    }
    inactive_hours(active).is_some_and(|hours| hours >= stale_after_hours as i64)
}

/// Folds a legacy single `~/.galdr/active` flag into the `active.d/` scheme, once.
/// Idempotent and concurrency-safe (atomic write, tolerant remove): a present,
/// parseable flag is rewritten as `active.d/<rec_id>.json` and the legacy file
/// removed; a corrupt or absent flag is a no-op (it holds no recording to preserve).
/// This is how an in-progress recording survives the upgrade to concurrent capture.
pub fn migrate_legacy_active() {
    let Ok(legacy) = paths::legacy_active_flag() else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&legacy) else {
        return; // absent (or unreadable): nothing to migrate.
    };
    let Ok(active) = serde_json::from_str::<ActiveRec>(&contents) else {
        return; // corrupt: no recording to preserve; leave it for `read_active_all` to ignore.
    };
    if write_active(&active).is_ok() {
        let _ = std::fs::remove_file(&legacy);
    }
}

/// Every active recording, newest first (`rec_id` is a time-sortable ULID). Folds a
/// legacy `active` flag in first, so no in-progress recording is dropped on upgrade.
pub fn read_active_all() -> Vec<ActiveRec> {
    migrate_legacy_active();
    let Ok(dir) = paths::active_dir() else {
        return Vec::new();
    };
    let mut actives: Vec<ActiveRec> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(contents) = std::fs::read_to_string(&path)
                && let Ok(active) = serde_json::from_str::<ActiveRec>(&contents)
            {
                actives.push(active);
            }
        }
    }
    actives.sort_by(|a, b| b.rec_id.cmp(&a.rec_id));
    actives
}

/// Writes (or overwrites) one recording's active flag under `active.d/`, atomically.
pub fn write_active(active: &ActiveRec) -> Result<()> {
    paths::ensure_dirs()?;
    let path = paths::active_file(&active.rec_id)?;
    write_atomic(&path, serde_json::to_string_pretty(active)?.as_bytes())
}

/// Drops one recording's active flag (best-effort; a missing file is fine).
fn remove_active(rec_id: &str) -> Result<()> {
    let path = paths::active_file(rec_id)?;
    let _ = std::fs::remove_file(path);
    Ok(())
}

/// Writes bytes to `path` atomically: a uniquely-named temp file in the same
/// directory, then a rename over the target. Two concurrent hooks (different sessions)
/// write different files, and the rename guarantees no reader ever sees a half-written
/// flag even if a hook is interrupted mid-write.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("active");
    let tmp = dir.join(format!(".{stem}.tmp.{}", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("could not write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("could not install {}", path.display()))?;
    Ok(())
}

/// Starts a recording. Concurrency-friendly: several can be active at once, each
/// scoped by the session that first acts under its `origin_cwd`. The recording begins
/// **unbound**; the sensor binds it to the first eligible session (see `hook`). If the
/// starting session already has a bound recording, this one simply waits unbound until
/// that one stops — the lock lives in the binding, not in `start`.
pub fn start(name: Option<String>) -> Result<()> {
    paths::ensure_dirs()?;
    migrate_legacy_active();

    // A forgotten `rec stop` must not haunt the machine: close anything stale before
    // opening a new recording, so a zombie neither swallows session-less events nor
    // scares agents (which read `rec status` before recording) away forever.
    let stale_after_hours = crate::config::Config::load_capture().stale_after_hours;
    for active in read_active_all() {
        if is_stale(&active, stale_after_hours) {
            let hours = inactive_hours(&active).unwrap_or_default();
            let _ = stop_one(
                &active,
                Some(format!(
                    "stale: inactive for {hours}h, auto-closed by rec start"
                )),
            );
        }
    }

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
    let others = read_active_all().len().saturating_sub(1);
    if others > 0 {
        println!(
            "  ({others} other recording(s) already active — this one binds to the session that next acts here)"
        );
    }
    println!("  do the task, then:  galdr rec stop [name]");
    Ok(())
}

/// Stops an active recording and writes its metadata. With `reference` (a name,
/// `rec_id`, or unique prefix) stops that one; without it, stops the sole active
/// recording, or errors listing them when several are active.
pub fn stop(reference: Option<&str>) -> Result<()> {
    let actives = read_active_all();
    if actives.is_empty() {
        bail!("no active recording");
    }
    let target = match reference.map(str::trim).filter(|s| !s.is_empty()) {
        Some(reference) => resolve_active(&actives, reference)?,
        None if actives.len() == 1 => actives.into_iter().next().unwrap(),
        None => {
            let list = actives
                .iter()
                .map(|a| format!("{} ({})", a.name, a.rec_id))
                .collect::<Vec<_>>()
                .join(", ");
            bail!("multiple recordings active — specify which: {list}");
        }
    };
    stop_one(&target, None)
}

/// Closes one active recording: persist the span, write its metadata, drop its flag,
/// and best-effort index it. `closed_reason` marks a non-manual close (e.g. a stale
/// reap) both in the printed line and in the recording's metadata.
fn stop_one(active: &ActiveRec, closed_reason: Option<String>) -> Result<()> {
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
        closed_reason,
    };
    paths::ensure_dirs()?;
    let rec_path = paths::recording_file(&active.rec_id)?;
    std::fs::write(&rec_path, serde_json::to_string_pretty(&recording)?)?;

    // Drop this recording's flag: from here on the sensor stops recording it.
    remove_active(&active.rec_id)?;

    // Keep the local catalog current even when the daemon is not running. This
    // is best-effort because the span + recording metadata are the truth.
    let _ = catalog::sync_closed_recording(&recording, &events);

    // Best-effort, after the metadata is on disk: tell the daemon to index it.
    crate::ipc::notify_best_effort(&crate::ipc::Request::RecordingClosed {
        recording: recording.clone(),
    });

    let plural = if steps == 1 { "" } else { "s" };
    match &recording.closed_reason {
        // A reaped recording is junk by definition — no distill invitation.
        Some(reason) => println!(
            "{} closed \"{}\" — {steps} step{plural} ({reason})",
            crate::style::dim("■"),
            active.name
        ),
        None => {
            println!(
                "{} stopped \"{}\" — {steps} step{plural}",
                crate::style::accent("■"),
                active.name
            );
            println!("  turn it into a skill:  galdr distill");
        }
    }
    Ok(())
}

/// Resolves a reference (exact `rec_id`, unique case-insensitive `rec_id` prefix, or
/// name) to one active recording — the [`resolve_in`] equivalent for the active set.
/// An ambiguous name is refused (unlike closed recordings, stopping the wrong live
/// recording is not recoverable), and misses point at `galdr rec status`.
fn resolve_active(actives: &[ActiveRec], reference: &str) -> Result<ActiveRec> {
    if let Some(active) = actives.iter().find(|a| a.rec_id == reference) {
        return Ok(active.clone());
    }
    let upper = reference.to_ascii_uppercase();
    let by_prefix: Vec<&ActiveRec> = actives
        .iter()
        .filter(|a| a.rec_id.starts_with(&upper))
        .collect();
    if by_prefix.len() == 1 {
        return Ok(by_prefix[0].clone());
    }
    if by_prefix.len() > 1 {
        bail!(
            "`{reference}` matches {} active recordings — add more characters, or use the name (see `galdr rec status`).",
            by_prefix.len()
        );
    }
    let by_name: Vec<&ActiveRec> = actives.iter().filter(|a| a.name == reference).collect();
    match by_name.as_slice() {
        [one] => Ok((*one).clone()),
        [] => bail!("no active recording matches `{reference}` — run `galdr rec status`."),
        many => bail!(
            "`{reference}` matches {} active recordings named that — use the rec_id (see `galdr rec status`).",
            many.len()
        ),
    }
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
            "{}  {}  {} steps  {}",
            crate::style::dim(&rec.rec_id),
            crate::style::accent(&format!("{:<20}", rec.name)),
            rec.steps,
            crate::style::dim(&rec.started_at),
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
            closed_reason: None,
        }
    }

    fn active(id: &str, started_at: &str) -> ActiveRec {
        ActiveRec {
            rec_id: id.into(),
            name: "t".into(),
            started_at: started_at.into(),
            transcript_path: None,
            origin_cwd: None,
            bound_session: None,
        }
    }

    #[test]
    fn staleness_follows_started_at_when_no_span_exists() {
        // The rec_id resolves to a span path that does not exist, so `started_at`
        // is the activity signal: a week-old start is stale, a fresh one is not.
        let old = active("01TESTSTALE0000000000000000", "2026-01-01T00:00:00Z");
        assert!(is_stale(&old, 24));
        let fresh = active("01TESTFRESH0000000000000000", &now_rfc3339());
        assert!(!is_stale(&fresh, 24));
    }

    #[test]
    fn staleness_zero_disables_and_unknown_activity_is_never_stale() {
        let old = active("01TESTSTALE0000000000000000", "2026-01-01T00:00:00Z");
        assert!(!is_stale(&old, 0), "0 disables staleness");
        // An unparseable started_at (and no span) means unknown activity: not stale.
        let unknown = active("01TESTUNKNOWN00000000000000", "not-a-timestamp");
        assert!(!is_stale(&unknown, 24));
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
