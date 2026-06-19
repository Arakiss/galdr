//! Queryable catalog over the spans and recordings.
//!
//! The catalog is a SQLite **index, never the source of truth**. Everything in it
//! can be thrown away and rebuilt from `spans/` + `recordings/` (+ the skills
//! directory for provenance) with [`reindex`]. Two consequences follow from that
//! stance:
//!
//! - Blobs are never stored. A step keeps only a one-line `summary`
//!   ([`crate::summary::summarize_input`]), not the raw `tool_input` /
//!   `tool_response`. That keeps the index small and keeps sensitive payloads out
//!   of a second on-disk copy.
//! - The schema is created idempotently and gated by `PRAGMA user_version`, so an
//!   `open()` against an old or partial database just works.
//!
//! `recordings` carries a placeholder row (rec_id only) the moment a step is
//! indexed, so steps can be indexed before a recording is closed without breaking
//! the foreign key; the metadata is filled in on close.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, MAIN_DB, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::record::Recording;
use crate::span::Event;

/// One row of the recordings list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingRow {
    pub rec_id: String,
    pub name: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub steps: i64,
    pub cwd: Option<String>,
    /// Whether a skill has been distilled from this recording.
    pub distilled: bool,
}

/// One step within a recording (the index view, no raw blobs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRow {
    pub seq: i64,
    pub tool_name: String,
    pub ts: String,
    pub summary: String,
}

/// A recording together with its steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingDetail {
    pub recording: RecordingRow,
    pub steps: Vec<StepRow>,
}

/// One installed skill and its provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRow {
    pub skill_name: String,
    pub rec_id: Option<String>,
    pub skill_path: String,
    pub installed_at: Option<String>,
    /// `true` if the skill's recording is missing (or it has no provenance).
    pub orphan: bool,
}

/// What a reindex rebuilt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReindexStats {
    pub recordings: usize,
    pub steps: usize,
    pub skills: usize,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS recordings (
    rec_id     TEXT PRIMARY KEY,
    name       TEXT,
    started_at TEXT,
    ended_at   TEXT,
    steps      INTEGER,
    cwd        TEXT
);
CREATE TABLE IF NOT EXISTS steps (
    rec_id    TEXT NOT NULL,
    seq       INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    ts        TEXT,
    summary   TEXT,
    PRIMARY KEY (rec_id, seq),
    FOREIGN KEY (rec_id) REFERENCES recordings(rec_id)
);
CREATE INDEX IF NOT EXISTS idx_steps_rec ON steps(rec_id, seq);
CREATE TABLE IF NOT EXISTS skills (
    skill_name   TEXT PRIMARY KEY,
    rec_id       TEXT,
    skill_path   TEXT,
    installed_at TEXT
);
";

fn apply_pragmas(conn: &Connection, wal: bool) -> Result<()> {
    if wal {
        // execute_batch tolerates the result row PRAGMA journal_mode returns.
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    }
    conn.execute_batch("PRAGMA busy_timeout=3000; PRAGMA foreign_keys=ON;")?;
    Ok(())
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", 1_i64)?;
    }
    Ok(())
}

/// Opens the live catalog (read/write, WAL), creating and migrating it if needed.
pub fn open() -> Result<Connection> {
    crate::paths::ensure_dirs()?;
    let path = crate::paths::catalog_db()?;
    let conn = Connection::open(&path)
        .with_context(|| format!("could not open catalog {}", path.display()))?;
    apply_pragmas(&conn, true)?;
    migrate_schema(&conn)?;
    Ok(conn)
}

/// Opens the catalog read-only. Used by the CLI as a fallback when the daemon is
/// down; if it fails (e.g. a WAL database with no readable side files) the caller
/// drops to the in-memory disk scan.
pub fn open_readonly() -> Result<Connection> {
    let path = crate::paths::catalog_db()?;
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("could not open catalog (read-only) {}", path.display()))?;
    conn.busy_timeout(Duration::from_millis(3000))?;
    Ok(conn)
}

/// Builds a throwaway in-memory catalog straight from disk. This is the CLI's last
/// structured fallback: no daemon, no usable database file, yet `list`/`show`/
/// `skills` still answer from the spans and recordings on disk.
pub fn open_in_memory_indexed() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    apply_pragmas(&conn, false)?;
    migrate_schema(&conn)?;
    reindex_into(&conn)?;
    Ok(conn)
}

/// Indexes one observed event. Inserts a placeholder recordings row first so the
/// foreign key holds even if the recording has not been closed yet.
pub fn index_event(conn: &Connection, rec_id: &str, event: &Event) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO recordings(rec_id) VALUES (?1)",
        params![rec_id],
    )?;
    let summary = crate::summary::summarize_input(&event.tool_name, &event.tool_input);
    conn.execute(
        "INSERT OR REPLACE INTO steps(rec_id, seq, tool_name, ts, summary) VALUES (?1,?2,?3,?4,?5)",
        params![rec_id, event.seq as i64, event.tool_name, event.ts, summary],
    )?;
    Ok(())
}

/// Fills (or updates) a recording's metadata on close.
pub fn index_recording(conn: &Connection, rec: &Recording) -> Result<()> {
    conn.execute(
        "INSERT INTO recordings(rec_id,name,started_at,ended_at,steps,cwd) VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(rec_id) DO UPDATE SET
           name=excluded.name, started_at=excluded.started_at,
           ended_at=excluded.ended_at, steps=excluded.steps, cwd=excluded.cwd",
        params![
            rec.rec_id,
            rec.name,
            rec.started_at,
            rec.ended_at,
            rec.steps as i64,
            rec.cwd,
        ],
    )?;
    Ok(())
}

/// Records that a skill was installed from a recording.
pub fn upsert_skill(
    conn: &Connection,
    skill_name: &str,
    rec_id: Option<&str>,
    skill_path: &str,
    installed_at: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO skills(skill_name, rec_id, skill_path, installed_at) VALUES(?1,?2,?3,?4)
         ON CONFLICT(skill_name) DO UPDATE SET
           rec_id=excluded.rec_id, skill_path=excluded.skill_path, installed_at=excluded.installed_at",
        params![skill_name, rec_id, skill_path, installed_at],
    )?;
    Ok(())
}

const RECORDING_SELECT: &str = "
SELECT rec_id, COALESCE(name,''), COALESCE(started_at,''), ended_at, COALESCE(steps,0), cwd,
       EXISTS(SELECT 1 FROM skills s WHERE s.rec_id = recordings.rec_id) AS distilled
FROM recordings";

fn map_recording(r: &rusqlite::Row<'_>) -> rusqlite::Result<RecordingRow> {
    Ok(RecordingRow {
        rec_id: r.get(0)?,
        name: r.get(1)?,
        started_at: r.get(2)?,
        ended_at: r.get(3)?,
        steps: r.get(4)?,
        cwd: r.get(5)?,
        distilled: r.get::<_, i64>(6)? != 0,
    })
}

/// Lists closed recordings, newest first.
pub fn list_recordings(conn: &Connection) -> Result<Vec<RecordingRow>> {
    let sql = format!("{RECORDING_SELECT} WHERE ended_at IS NOT NULL ORDER BY rec_id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_recording)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Shows one recording with its steps. `None` if the id is unknown.
pub fn show_recording(conn: &Connection, id: &str) -> Result<Option<RecordingDetail>> {
    let sql = format!("{RECORDING_SELECT} WHERE rec_id = ?1");
    let recording = conn
        .query_row(&sql, params![id], map_recording)
        .optional()?;
    let Some(recording) = recording else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT seq, tool_name, COALESCE(ts,''), COALESCE(summary,'')
         FROM steps WHERE rec_id = ?1 ORDER BY seq",
    )?;
    let steps = stmt
        .query_map(params![id], |r| {
            Ok(StepRow {
                seq: r.get(0)?,
                tool_name: r.get(1)?,
                ts: r.get(2)?,
                summary: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(RecordingDetail { recording, steps }))
}

/// Lists installed skills with provenance, flagging orphans.
pub fn list_skills(conn: &Connection) -> Result<Vec<SkillRow>> {
    let mut stmt = conn.prepare(
        "SELECT skill_name, rec_id, COALESCE(skill_path,''), installed_at,
                (NOT EXISTS(SELECT 1 FROM recordings r
                            WHERE r.rec_id = skills.rec_id AND r.ended_at IS NOT NULL)) AS orphan
         FROM skills ORDER BY skill_name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(SkillRow {
            skill_name: r.get(0)?,
            rec_id: r.get(1)?,
            skill_path: r.get(2)?,
            installed_at: r.get(3)?,
            orphan: r.get::<_, i64>(4)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Best-effort reconciliation run by the daemon's poll-watcher: pull in any events
/// that a dropped notification left unindexed. Idempotent and bounded — it only
/// touches the active recording's tail and closed recordings not yet present.
pub fn reconcile(conn: &Connection) -> Result<()> {
    if let Some(active) = crate::record::read_active() {
        let span_path = crate::paths::span_file(&active.rec_id)?;
        let stored: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM steps WHERE rec_id = ?1",
            params![active.rec_id],
            |r| r.get(0),
        )?;
        for ev in crate::span::read_span(&span_path)
            .unwrap_or_default()
            .iter()
            .filter(|e| e.seq as i64 > stored)
        {
            index_event(conn, &active.rec_id, ev)?;
        }
    }

    let dir = crate::paths::recordings_dir()?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(rec) = serde_json::from_str::<Recording>(&contents) else {
                continue;
            };
            let present: i64 = conn.query_row(
                "SELECT COUNT(*) FROM recordings WHERE rec_id = ?1 AND ended_at IS NOT NULL",
                params![rec.rec_id],
                |r| r.get(0),
            )?;
            if present == 0 {
                let span_path = crate::paths::span_file(&rec.rec_id)?;
                for ev in crate::span::read_span(&span_path).unwrap_or_default() {
                    index_event(conn, &rec.rec_id, &ev)?;
                }
                index_recording(conn, &rec)?;
            }
        }
    }
    Ok(())
}

/// Rebuilds the catalog content into `conn` from the on-disk spans, recordings,
/// and skills. Wrapped in a single transaction for speed.
pub fn reindex_into(conn: &Connection) -> Result<ReindexStats> {
    reindex_into_dirs(
        conn,
        &crate::paths::spans_dir()?,
        &crate::paths::recordings_dir()?,
        &crate::paths::skills_root()?,
    )
}

fn reindex_into_dirs(
    conn: &Connection,
    spans_dir: &Path,
    recordings_dir: &Path,
    skills_root: &Path,
) -> Result<ReindexStats> {
    let tx = conn.unchecked_transaction()?;
    let mut stats = ReindexStats::default();

    // 1) Every span event (creates FK-safe placeholder recordings as it goes).
    if let Ok(entries) = std::fs::read_dir(spans_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(rec_id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            for ev in crate::span::read_span(&path).unwrap_or_default() {
                index_event(&tx, rec_id, &ev)?;
                stats.steps += 1;
            }
        }
    }

    // 2) Recording metadata.
    if let Ok(entries) = std::fs::read_dir(recordings_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(rec) = serde_json::from_str::<Recording>(&contents) else {
                continue;
            };
            index_recording(&tx, &rec)?;
            stats.recordings += 1;
        }
    }

    // 3) Skills provenance.
    for (name, rec_id, path) in scan_skills_raw(skills_root) {
        upsert_skill(&tx, &name, rec_id.as_deref(), &path, None)?;
        stats.skills += 1;
    }

    tx.commit()?;
    Ok(stats)
}

/// Atomically rebuilds the live catalog: populate a fresh temp database, then
/// restore it over the live connection in place (keeping the connection valid and
/// readers consistent). The temp file uses a rollback journal so it is a complete
/// database the moment it is dropped, before the restore reads it.
pub fn reindex(live: &mut Connection) -> Result<ReindexStats> {
    crate::paths::ensure_dirs()?;
    let temp_path = crate::paths::galdr_root()?.join("catalog.rebuild.sqlite");
    let _ = std::fs::remove_file(&temp_path);

    let stats = {
        let temp = Connection::open(&temp_path)?;
        apply_pragmas(&temp, false)?;
        migrate_schema(&temp)?;
        let stats = reindex_into(&temp)?;
        drop(temp);
        stats
    };

    live.restore(MAIN_DB, &temp_path, None::<fn(rusqlite::backup::Progress)>)
        .context("failed to restore the rebuilt catalog over the live database")?;
    let _ = std::fs::remove_file(&temp_path);
    Ok(stats)
}

/// Scans the skills directory for `(skill_name, rec_id, skill_path)`. Tolerant:
/// an unreadable or unparseable skill is skipped, and a missing provenance line
/// just yields `None` for the rec_id (which the catalog flags as an orphan).
fn scan_skills_raw(skills_root: &Path) -> Vec<(String, Option<String>, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(skills_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        let Ok(contents) = std::fs::read_to_string(&skill_md) else {
            continue;
        };
        let name = parse_frontmatter_name(&contents)
            .or_else(|| dir.file_name().and_then(|s| s.to_str()).map(str::to_string))
            .unwrap_or_else(|| "skill".to_string());
        let rec_id = parse_rec_id(&contents);
        out.push((name, rec_id, skill_md.to_string_lossy().into_owned()));
    }
    out
}

fn parse_frontmatter_name(md: &str) -> Option<String> {
    let mut lines = md.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn parse_rec_id(md: &str) -> Option<String> {
    for line in md.lines() {
        if let Some(idx) = line.find("rec_id:") {
            let after = &line[idx + "rec_id:".len()..];
            let token = after
                .trim()
                .trim_matches('`')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('`');
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn, false).unwrap();
        migrate_schema(&conn).unwrap();
        conn
    }

    fn event(seq: u64, tool: &str) -> Event {
        Event {
            ts: "2026-06-19T00:00:00Z".into(),
            seq,
            tool_name: tool.into(),
            tool_input: serde_json::json!({ "command": "echo hi" }),
            tool_response: serde_json::json!({}),
            cwd: Some("/tmp".into()),
            session_id: None,
        }
    }

    fn recording(id: &str, steps: usize) -> Recording {
        Recording {
            rec_id: id.into(),
            name: "demo".into(),
            started_at: "2026-06-19T00:00:00Z".into(),
            ended_at: "2026-06-19T00:01:00Z".into(),
            steps,
            cwd: Some("/tmp".into()),
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = mem();
        // Running it again must not error and must keep the version pinned.
        migrate_schema(&conn).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn index_and_list_roundtrip() {
        let conn = mem();
        // A step indexed before close creates only a placeholder (no ended_at), so
        // it must not appear in the closed-recordings list yet.
        index_event(&conn, "01AAA", &event(0, "Bash")).unwrap();
        assert!(list_recordings(&conn).unwrap().is_empty());

        index_recording(&conn, &recording("01AAA", 1)).unwrap();
        let rows = list_recordings(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rec_id, "01AAA");
        assert!(!rows[0].distilled);

        let detail = show_recording(&conn, "01AAA").unwrap().unwrap();
        assert_eq!(detail.steps.len(), 1);
        assert_eq!(detail.steps[0].tool_name, "Bash");
        assert_eq!(detail.steps[0].summary, "echo hi");

        upsert_skill(&conn, "galdr-demo", Some("01AAA"), "/x/SKILL.md", None).unwrap();
        assert!(list_recordings(&conn).unwrap()[0].distilled);
        let skills = list_skills(&conn).unwrap();
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].orphan);

        // A skill pointing at an unknown recording is an orphan.
        upsert_skill(&conn, "galdr-ghost", Some("01ZZZ"), "/y/SKILL.md", None).unwrap();
        let ghost = list_skills(&conn)
            .unwrap()
            .into_iter()
            .find(|s| s.skill_name == "galdr-ghost")
            .unwrap();
        assert!(ghost.orphan);
    }

    #[test]
    fn reindex_rebuilds_from_disk() {
        let root = tempfile::tempdir().unwrap();
        let spans = root.path().join("spans");
        let recordings = root.path().join("recordings");
        let skills = root.path().join("skills");
        std::fs::create_dir_all(&spans).unwrap();
        std::fs::create_dir_all(&recordings).unwrap();
        std::fs::create_dir_all(skills.join("galdr-demo")).unwrap();

        crate::span::append_event(&spans.join("01AAA.jsonl"), &event(0, "Bash")).unwrap();
        crate::span::append_event(&spans.join("01AAA.jsonl"), &event(1, "Write")).unwrap();
        std::fs::write(
            recordings.join("01AAA.json"),
            serde_json::to_string(&recording("01AAA", 2)).unwrap(),
        )
        .unwrap();
        std::fs::write(
            skills.join("galdr-demo").join("SKILL.md"),
            "---\nname: galdr-demo\n---\n\n## Provenance\n\n- rec_id: `01AAA`\n",
        )
        .unwrap();

        let conn = mem();
        let stats = reindex_into_dirs(&conn, &spans, &recordings, &skills).unwrap();
        assert_eq!(stats.recordings, 1);
        assert_eq!(stats.steps, 2);
        assert_eq!(stats.skills, 1);

        let rows = list_recordings(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].distilled,
            "provenance links the skill to its recording"
        );
        let skills = list_skills(&conn).unwrap();
        assert_eq!(skills[0].rec_id.as_deref(), Some("01AAA"));
        assert!(!skills[0].orphan);
    }
}
