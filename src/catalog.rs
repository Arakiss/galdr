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

use crate::outcome::{SkillOutcomeEvent, SkillUsageEvent};
use crate::record::Recording;
use crate::span::Event;

pub const STATUS_DRAFT: &str = "draft";
pub const STATUS_FINAL: &str = "final";
pub const STATUS_PARAM_DRAFT: &str = "param-draft";
pub const STATUS_UNKNOWN: &str = "unknown";

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
    pub status: String,
    pub readiness_score: i64,
    pub readiness_delta: i64,
    pub readiness_notes: String,
    /// `true` if the skill's recording is missing (or it has no provenance).
    pub orphan: bool,
}

/// One evaluator output for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEvaluationRow {
    pub id: i64,
    pub skill_name: String,
    pub skill_path: String,
    pub rec_id: Option<String>,
    pub evaluator_kind: String,
    pub score: i64,
    pub confidence: f64,
    pub score_delta: i64,
    pub rationale_json: String,
    pub evidence_refs: String,
    pub created_at: String,
}

/// One observed use of a skill in a later recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageRow {
    pub event_id: String,
    pub skill_name: String,
    pub skill_hash: Option<String>,
    pub rec_id: String,
    pub task_kind: Option<String>,
    pub outcome: String,
    pub retries: i64,
    pub manual_intervention_count: i64,
    pub notes: Option<String>,
    pub created_at: String,
}

/// One explicit label or review attached to a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillOutcomeRow {
    pub event_id: String,
    pub skill_name: String,
    pub rec_id: Option<String>,
    pub evaluator_kind: String,
    pub label: String,
    pub confidence: f64,
    pub notes: Option<String>,
    pub evidence_refs: String,
    pub created_at: String,
}

/// What a reindex rebuilt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReindexStats {
    pub recordings: usize,
    pub steps: usize,
    pub skills: usize,
    pub usages: usize,
    pub outcomes: usize,
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

const SKILL_EVALUATIONS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS skill_evaluations (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    skill_name     TEXT NOT NULL,
    skill_path     TEXT NOT NULL,
    rec_id         TEXT,
    evaluator_kind TEXT NOT NULL,
    score          INTEGER NOT NULL,
    confidence     REAL NOT NULL,
    score_delta    INTEGER NOT NULL DEFAULT 0,
    rationale_json TEXT NOT NULL,
    evidence_refs  TEXT NOT NULL,
    created_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_skill_evaluations_skill
    ON skill_evaluations(skill_name, id);
CREATE INDEX IF NOT EXISTS idx_skill_evaluations_kind
    ON skill_evaluations(evaluator_kind, id);
";

const EVALUATOR_READINESS_LINT: &str = "readiness_lint";

const SKILL_OUTCOME_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS skill_usage (
    event_id                  TEXT PRIMARY KEY,
    skill_name                TEXT NOT NULL,
    skill_hash                TEXT,
    rec_id                    TEXT NOT NULL,
    task_kind                 TEXT,
    outcome                   TEXT NOT NULL,
    retries                   INTEGER NOT NULL,
    manual_intervention_count INTEGER NOT NULL,
    notes                     TEXT,
    created_at                TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_skill_usage_skill
    ON skill_usage(skill_name, created_at);
CREATE INDEX IF NOT EXISTS idx_skill_usage_rec
    ON skill_usage(rec_id);

CREATE TABLE IF NOT EXISTS skill_outcomes (
    event_id       TEXT PRIMARY KEY,
    skill_name     TEXT NOT NULL,
    rec_id         TEXT,
    evaluator_kind TEXT NOT NULL,
    label          TEXT NOT NULL,
    confidence     REAL NOT NULL,
    notes          TEXT,
    evidence_refs  TEXT NOT NULL,
    created_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_skill_outcomes_skill
    ON skill_outcomes(skill_name, created_at);
CREATE INDEX IF NOT EXISTS idx_skill_outcomes_rec
    ON skill_outcomes(rec_id);
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
    if version < 2 {
        add_column_if_missing(conn, "skills", "status", "TEXT NOT NULL DEFAULT 'unknown'")?;
        conn.pragma_update(None, "user_version", 2_i64)?;
    }
    if version < 3 {
        add_column_if_missing(
            conn,
            "skills",
            "readiness_score",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            conn,
            "skills",
            "readiness_delta",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        add_column_if_missing(
            conn,
            "skills",
            "readiness_notes",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        copy_legacy_quality_columns(conn)?;
        conn.execute_batch(SKILL_EVALUATIONS_SCHEMA)?;
        conn.pragma_update(None, "user_version", 3_i64)?;
    }
    if version < 4 {
        conn.execute_batch(SKILL_OUTCOME_SCHEMA)?;
        conn.pragma_update(None, "user_version", 4_i64)?;
    }
    Ok(())
}

fn copy_legacy_quality_columns(conn: &Connection) -> Result<()> {
    if has_column(conn, "skills", "quality_score")? {
        conn.execute_batch(
            "UPDATE skills
             SET readiness_score = COALESCE(quality_score, readiness_score);",
        )?;
    }
    if has_column(conn, "skills", "quality_delta")? {
        conn.execute_batch(
            "UPDATE skills
             SET readiness_delta = COALESCE(quality_delta, readiness_delta);",
        )?;
    }
    if has_column(conn, "skills", "quality_notes")? {
        conn.execute_batch(
            "UPDATE skills
             SET readiness_notes = COALESCE(NULLIF(quality_notes, ''), readiness_notes);",
        )?;
    }
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    if has_column(conn, table, column)? {
        return Ok(());
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {table} ADD COLUMN {column} {definition};"
    ))?;
    Ok(())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
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
    status: &str,
) -> Result<()> {
    let readiness = analyze_skill_file(skill_path, rec_id);
    let previous_score = conn
        .query_row(
            "SELECT readiness_score FROM skills WHERE skill_name = ?1",
            params![skill_name],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    let readiness_delta = previous_score.map_or(0, |old| readiness.score - old);
    conn.execute(
        "INSERT INTO skills(skill_name, rec_id, skill_path, installed_at, status, readiness_score, readiness_delta, readiness_notes)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(skill_name) DO UPDATE SET
           rec_id=excluded.rec_id,
           skill_path=excluded.skill_path,
           installed_at=excluded.installed_at,
           status=excluded.status,
           readiness_score=excluded.readiness_score,
           readiness_delta=excluded.readiness_delta,
           readiness_notes=excluded.readiness_notes",
        params![
            skill_name,
            rec_id,
            skill_path,
            installed_at,
            status,
            readiness.score,
            readiness_delta,
            readiness.notes
        ],
    )?;
    record_readiness_evaluation(
        conn,
        skill_name,
        rec_id,
        skill_path,
        &readiness,
        readiness_delta,
    )?;
    Ok(())
}

fn record_readiness_evaluation(
    conn: &Connection,
    skill_name: &str,
    rec_id: Option<&str>,
    skill_path: &str,
    readiness: &SkillReadiness,
    readiness_delta: i64,
) -> Result<()> {
    let rationale_json = serde_json::to_string(&readiness.rationale)?;
    let evidence_refs = serde_json::to_string(&serde_json::json!({
        "skill_path": skill_path,
        "rec_id": rec_id,
    }))?;

    let previous = conn
        .query_row(
            "SELECT score, rationale_json, evidence_refs
             FROM skill_evaluations
             WHERE skill_name = ?1 AND evaluator_kind = ?2
             ORDER BY id DESC LIMIT 1",
            params![skill_name, EVALUATOR_READINESS_LINT],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((score, rationale, evidence)) = previous
        && score == readiness.score
        && rationale == rationale_json
        && evidence == evidence_refs
    {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO skill_evaluations(
            skill_name, skill_path, rec_id, evaluator_kind, score, confidence,
            score_delta, rationale_json, evidence_refs, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            skill_name,
            skill_path,
            rec_id,
            EVALUATOR_READINESS_LINT,
            readiness.score,
            readiness.confidence,
            readiness_delta,
            rationale_json,
            evidence_refs,
            crate::record::now_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Updates the live catalog for a closed recording. This is used by interactive
/// write paths when no daemon is running, so an existing catalog does not become
/// a stale fallback. The span remains the source of truth.
pub fn sync_closed_recording(rec: &Recording, events: &[Event]) -> Result<()> {
    let conn = open()?;
    let tx = conn.unchecked_transaction()?;
    for event in events {
        index_event(&tx, &rec.rec_id, event)?;
    }
    index_recording(&tx, rec)?;
    tx.commit()?;
    Ok(())
}

/// Updates the live catalog for an installed skill. This mirrors the daemon's
/// `SkillInstalled` handler for the no-daemon path.
pub fn sync_installed_skill(
    skill_name: &str,
    rec_id: Option<&str>,
    skill_path: &str,
    installed_at: Option<&str>,
    status: &str,
) -> Result<()> {
    let conn = open()?;
    upsert_skill(&conn, skill_name, rec_id, skill_path, installed_at, status)
}

/// Indexes one durable skill usage event.
pub fn index_skill_usage(conn: &Connection, event: &SkillUsageEvent) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO skill_usage(
            event_id, skill_name, skill_hash, rec_id, task_kind, outcome, retries,
            manual_intervention_count, notes, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            event.event_id,
            event.skill_name,
            event.skill_hash,
            event.rec_id,
            event.task_kind,
            event.outcome,
            i64::from(event.retries),
            i64::from(event.manual_intervention_count),
            event.notes,
            event.created_at,
        ],
    )?;
    Ok(())
}

/// Indexes one durable skill outcome label.
pub fn index_skill_outcome(conn: &Connection, event: &SkillOutcomeEvent) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO skill_outcomes(
            event_id, skill_name, rec_id, evaluator_kind, label, confidence,
            notes, evidence_refs, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            event.event_id,
            event.skill_name,
            event.rec_id,
            event.evaluator_kind,
            event.label,
            event.confidence,
            event.notes,
            serde_json::to_string(&event.evidence_refs)?,
            event.created_at,
        ],
    )?;
    Ok(())
}

/// Keeps the live catalog current for an appended usage event.
pub fn sync_skill_usage(event: &SkillUsageEvent) -> Result<()> {
    let conn = open()?;
    index_skill_usage(&conn, event)
}

/// Keeps the live catalog current for an appended outcome event.
pub fn sync_skill_outcome(event: &SkillOutcomeEvent) -> Result<()> {
    let conn = open()?;
    index_skill_outcome(&conn, event)
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
                COALESCE(status,'unknown'), COALESCE(readiness_score,0),
                COALESCE(readiness_delta,0), COALESCE(readiness_notes,''),
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
            status: r.get(4)?,
            readiness_score: r.get(5)?,
            readiness_delta: r.get(6)?,
            readiness_notes: r.get(7)?,
            orphan: r.get::<_, i64>(8)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Lists skill evaluations, newest first. Evaluator outputs are separate from
/// the skill row so deterministic lint, human review, LLM review, outcome
/// evidence, and future learned models can coexist without pretending to be the
/// same signal.
pub fn list_skill_evaluations(
    conn: &Connection,
    skill_name: Option<&str>,
) -> Result<Vec<SkillEvaluationRow>> {
    let map = |r: &rusqlite::Row<'_>| {
        Ok(SkillEvaluationRow {
            id: r.get(0)?,
            skill_name: r.get(1)?,
            skill_path: r.get(2)?,
            rec_id: r.get(3)?,
            evaluator_kind: r.get(4)?,
            score: r.get(5)?,
            confidence: r.get(6)?,
            score_delta: r.get(7)?,
            rationale_json: r.get(8)?,
            evidence_refs: r.get(9)?,
            created_at: r.get(10)?,
        })
    };

    if let Some(skill_name) = skill_name {
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, skill_path, rec_id, evaluator_kind, score,
                    confidence, score_delta, rationale_json, evidence_refs, created_at
             FROM skill_evaluations
             WHERE skill_name = ?1
             ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![skill_name], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, skill_path, rec_id, evaluator_kind, score,
                    confidence, score_delta, rationale_json, evidence_refs, created_at
             FROM skill_evaluations
             ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Lists observed skill usage events, newest first.
pub fn list_skill_usage(conn: &Connection, skill_name: Option<&str>) -> Result<Vec<SkillUsageRow>> {
    let map = |r: &rusqlite::Row<'_>| {
        Ok(SkillUsageRow {
            event_id: r.get(0)?,
            skill_name: r.get(1)?,
            skill_hash: r.get(2)?,
            rec_id: r.get(3)?,
            task_kind: r.get(4)?,
            outcome: r.get(5)?,
            retries: r.get(6)?,
            manual_intervention_count: r.get(7)?,
            notes: r.get(8)?,
            created_at: r.get(9)?,
        })
    };

    if let Some(skill_name) = skill_name {
        let mut stmt = conn.prepare(
            "SELECT event_id, skill_name, skill_hash, rec_id, task_kind, outcome,
                    retries, manual_intervention_count, notes, created_at
             FROM skill_usage
             WHERE skill_name = ?1
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map(params![skill_name], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        let mut stmt = conn.prepare(
            "SELECT event_id, skill_name, skill_hash, rec_id, task_kind, outcome,
                    retries, manual_intervention_count, notes, created_at
             FROM skill_usage
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map([], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Lists explicit skill outcome labels, newest first.
pub fn list_skill_outcomes(
    conn: &Connection,
    skill_name: Option<&str>,
) -> Result<Vec<SkillOutcomeRow>> {
    let map = |r: &rusqlite::Row<'_>| {
        Ok(SkillOutcomeRow {
            event_id: r.get(0)?,
            skill_name: r.get(1)?,
            rec_id: r.get(2)?,
            evaluator_kind: r.get(3)?,
            label: r.get(4)?,
            confidence: r.get(5)?,
            notes: r.get(6)?,
            evidence_refs: r.get(7)?,
            created_at: r.get(8)?,
        })
    };

    if let Some(skill_name) = skill_name {
        let mut stmt = conn.prepare(
            "SELECT event_id, skill_name, rec_id, evaluator_kind, label,
                    confidence, notes, evidence_refs, created_at
             FROM skill_outcomes
             WHERE skill_name = ?1
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map(params![skill_name], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        let mut stmt = conn.prepare(
            "SELECT event_id, skill_name, rec_id, evaluator_kind, label,
                    confidence, notes, evidence_refs, created_at
             FROM skill_outcomes
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map([], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
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
        &crate::paths::outcomes_dir()?,
    )
}

fn reindex_into_dirs(
    conn: &Connection,
    spans_dir: &Path,
    recordings_dir: &Path,
    skills_root: &Path,
    outcomes_dir: &Path,
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
    for skill in scan_skills_raw(skills_root) {
        upsert_skill(
            &tx,
            &skill.name,
            skill.rec_id.as_deref(),
            &skill.path,
            None,
            &skill.status,
        )?;
        stats.skills += 1;
    }

    let usage_log = outcomes_dir.join("skill_usage.jsonl");
    for event in crate::outcome::read_usage_log(&usage_log)? {
        index_skill_usage(&tx, &event)?;
        stats.usages += 1;
    }

    let outcome_log = outcomes_dir.join("skill_outcomes.jsonl");
    for event in crate::outcome::read_outcome_log(&outcome_log)? {
        index_skill_outcome(&tx, &event)?;
        stats.outcomes += 1;
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
struct SkillScan {
    name: String,
    rec_id: Option<String>,
    path: String,
    status: String,
}

fn scan_skills_raw(skills_root: &Path) -> Vec<SkillScan> {
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
        let status = infer_skill_status(&contents);
        out.push(SkillScan {
            name,
            rec_id,
            path: skill_md.to_string_lossy().into_owned(),
            status,
        });
    }
    out
}

struct SkillReadiness {
    score: i64,
    notes: String,
    confidence: f64,
    rationale: serde_json::Value,
}

fn analyze_skill_file(skill_path: &str, rec_id: Option<&str>) -> SkillReadiness {
    let Ok(contents) = std::fs::read_to_string(skill_path) else {
        return SkillReadiness {
            score: 0,
            notes: "skill file unreadable".to_string(),
            confidence: 0.2,
            rationale: serde_json::json!({
                "metric": "readiness_lint_v1",
                "error": "skill file unreadable"
            }),
        };
    };
    analyze_skill_md(&contents, rec_id)
}

fn analyze_skill_md(md: &str, rec_id: Option<&str>) -> SkillReadiness {
    let mut score = 100_i64;
    let mut notes = Vec::new();

    let has_name = has_frontmatter_name(md);
    let has_description = has_frontmatter_description(md);
    if !has_name {
        score -= 20;
        notes.push("missing frontmatter name");
    }
    if !has_description {
        score -= 20;
        notes.push("missing frontmatter description");
    }
    let mut missing_sections = Vec::new();
    for section in ["Goal", "Procedure", "Success criteria"] {
        if !has_section(md, section) {
            missing_sections.push(section);
        }
    }
    if !missing_sections.is_empty() {
        score -= 10 * missing_sections.len() as i64;
        notes.push("missing required sections");
    }
    let draft_markers_present = md.contains("TODO(agent)") || md.contains("[galdr DRAFT]");
    if draft_markers_present {
        score -= 25;
        notes.push("draft markers present");
    }
    let provenance_present = rec_id.is_some();
    if !provenance_present {
        score -= 10;
        notes.push("missing provenance");
    }

    SkillReadiness {
        score: score.max(0),
        notes: if notes.is_empty() {
            "ready".to_string()
        } else {
            notes.join("; ")
        },
        confidence: 0.95,
        rationale: serde_json::json!({
            "metric": "readiness_lint_v1",
            "frontmatter": {
                "name": has_name,
                "description": has_description
            },
            "required_sections": {
                "missing": missing_sections
            },
            "draft_markers_present": draft_markers_present,
            "provenance_present": provenance_present
        }),
    }
}

pub fn infer_skill_status(md: &str) -> String {
    if md.contains("Parametrized from two recordings") || md.contains("## Procedure (parametrized)")
    {
        return STATUS_PARAM_DRAFT.to_string();
    }
    if md.contains("TODO(agent)") || md.contains("[galdr DRAFT]") {
        return STATUS_DRAFT.to_string();
    }
    if has_section(md, "Goal")
        && has_section(md, "Procedure")
        && has_section(md, "Success criteria")
    {
        return STATUS_FINAL.to_string();
    }
    STATUS_UNKNOWN.to_string()
}

fn has_frontmatter_name(md: &str) -> bool {
    parse_frontmatter_name(md).is_some()
}

fn has_frontmatter_description(md: &str) -> bool {
    let mut lines = md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.starts_with("description:") {
            return true;
        }
    }
    false
}

fn has_section(md: &str, section: &str) -> bool {
    md.lines()
        .any(|line| line.trim().eq_ignore_ascii_case(&format!("## {section}")))
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
        assert_eq!(version, 4);
    }

    #[test]
    fn migrate_adds_skill_readiness_columns_and_evaluations_to_v1_catalogs() {
        let conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn, false).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.pragma_update(None, "user_version", 1_i64).unwrap();

        migrate_schema(&conn).unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info(skills)").unwrap();
        let columns = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(columns.contains(&"status".to_string()));
        assert!(columns.contains(&"readiness_score".to_string()));
        assert!(columns.contains(&"readiness_delta".to_string()));
        assert!(columns.contains(&"readiness_notes".to_string()));

        let eval_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='skill_evaluations'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(eval_count, 1);
        let usage_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='skill_usage'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(usage_count, 1);
        let outcome_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='skill_outcomes'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outcome_count, 1);
    }

    #[test]
    fn migrate_copies_legacy_quality_columns_to_readiness_columns() {
        let conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn, false).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "ALTER TABLE skills ADD COLUMN status TEXT NOT NULL DEFAULT 'unknown';
             ALTER TABLE skills ADD COLUMN quality_score INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE skills ADD COLUMN quality_delta INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE skills ADD COLUMN quality_notes TEXT NOT NULL DEFAULT '';
             INSERT INTO skills(skill_name, skill_path, status, quality_score, quality_delta, quality_notes)
             VALUES('legacy', '/x/SKILL.md', 'draft', 55, -5, 'legacy notes');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 2_i64).unwrap();

        migrate_schema(&conn).unwrap();

        let skill = list_skills(&conn).unwrap().remove(0);
        assert_eq!(skill.readiness_score, 55);
        assert_eq!(skill.readiness_delta, -5);
        assert_eq!(skill.readiness_notes, "legacy notes");
    }

    #[test]
    fn index_and_list_skill_usage_and_outcomes() {
        let conn = mem();
        let usage = SkillUsageEvent {
            event_id: "01USE".into(),
            skill_name: "galdr-demo".into(),
            skill_hash: Some("fnv1a64:abc".into()),
            rec_id: "01AAA".into(),
            task_kind: Some("smoke".into()),
            outcome: "success".into(),
            retries: 1,
            manual_intervention_count: 2,
            notes: Some("worked".into()),
            created_at: "2026-06-19T00:00:00Z".into(),
        };
        index_skill_usage(&conn, &usage).unwrap();

        let outcome = SkillOutcomeEvent {
            event_id: "01OUT".into(),
            skill_name: "galdr-demo".into(),
            rec_id: Some("01AAA".into()),
            evaluator_kind: "human".into(),
            label: "accepted".into(),
            confidence: 0.9,
            notes: Some("reviewed".into()),
            evidence_refs: serde_json::json!({"rec_id": "01AAA"}),
            created_at: "2026-06-19T00:01:00Z".into(),
        };
        index_skill_outcome(&conn, &outcome).unwrap();

        let usages = list_skill_usage(&conn, Some("galdr-demo")).unwrap();
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].outcome, "success");
        assert_eq!(usages[0].manual_intervention_count, 2);

        let outcomes = list_skill_outcomes(&conn, Some("galdr-demo")).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].label, "accepted");
        assert_eq!(outcomes[0].evaluator_kind, "human");
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

        upsert_skill(
            &conn,
            "galdr-demo",
            Some("01AAA"),
            "/x/SKILL.md",
            None,
            STATUS_FINAL,
        )
        .unwrap();
        assert!(list_recordings(&conn).unwrap()[0].distilled);
        let skills = list_skills(&conn).unwrap();
        assert_eq!(skills.len(), 1);
        assert!(!skills[0].orphan);
        let evals = list_skill_evaluations(&conn, Some("galdr-demo")).unwrap();
        assert_eq!(evals.len(), 1);
        assert_eq!(evals[0].evaluator_kind, EVALUATOR_READINESS_LINT);

        // A skill pointing at an unknown recording is an orphan.
        upsert_skill(
            &conn,
            "galdr-ghost",
            Some("01ZZZ"),
            "/y/SKILL.md",
            None,
            STATUS_FINAL,
        )
        .unwrap();
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
        let outcomes = root.path().join("outcomes");
        std::fs::create_dir_all(&spans).unwrap();
        std::fs::create_dir_all(&recordings).unwrap();
        std::fs::create_dir_all(skills.join("galdr-demo")).unwrap();
        std::fs::create_dir_all(&outcomes).unwrap();

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
        let stats = reindex_into_dirs(&conn, &spans, &recordings, &skills, &outcomes).unwrap();
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
