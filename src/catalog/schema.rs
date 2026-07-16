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
	    event_kind TEXT NOT NULL DEFAULT 'tool_call',
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

const STEP_JUDGMENT_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS step_judgments (
    event_id         TEXT PRIMARY KEY,
    rec_id           TEXT NOT NULL,
    seq              INTEGER NOT NULL,
    verdict          TEXT NOT NULL,
    rationale        TEXT NOT NULL,
    suggested_action TEXT,
    judge            TEXT,
    task_key         TEXT NOT NULL,
    created_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_step_judgments_rec_seq
    ON step_judgments(rec_id, seq, created_at);
CREATE INDEX IF NOT EXISTS idx_step_judgments_task
    ON step_judgments(task_key, seq);
";

const REGRESSION_BASE_CASE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS regression_base_cases (
    event_id          TEXT PRIMARY KEY,
    skill_name        TEXT NOT NULL,
    skill_hash        TEXT,
    rec_id            TEXT NOT NULL,
    case_name         TEXT NOT NULL,
    reference_version TEXT,
    notes             TEXT,
    created_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_regression_base_cases_skill
    ON regression_base_cases(skill_name, created_at);
CREATE INDEX IF NOT EXISTS idx_regression_base_cases_rec
    ON regression_base_cases(rec_id);
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
    if version < 5 {
        add_column_if_missing(
            conn,
            "steps",
            "event_kind",
            "TEXT NOT NULL DEFAULT 'tool_call'",
        )?;
        conn.pragma_update(None, "user_version", 5_i64)?;
    }
    if version < 6 {
        conn.execute_batch(STEP_JUDGMENT_SCHEMA)?;
        conn.pragma_update(None, "user_version", 6_i64)?;
    }
    if version < 7 {
        conn.execute_batch(REGRESSION_BASE_CASE_SCHEMA)?;
        conn.pragma_update(None, "user_version", 7_i64)?;
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
