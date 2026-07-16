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
    let summary = crate::summary::summarize_event(event);
    conn.execute(
        "INSERT OR REPLACE INTO steps(rec_id, seq, tool_name, event_kind, ts, summary) VALUES (?1,?2,?3,?4,?5,?6)",
        params![rec_id, event.seq as i64, event.tool_name, event.event_kind.as_str(), event.ts, summary],
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
    // The delta is read-modify-write against the previous score, so the SELECT and
    // the upsert must be one atomic step, or a concurrent writer (a CLI invocation
    // racing the daemon) could slip between them and make the delta stale. When the
    // caller already holds a transaction (e.g. `reindex`), reuse it — SQLite has no
    // nested transactions — otherwise open our own.
    if conn.is_autocommit() {
        let tx = conn.unchecked_transaction()?;
        upsert_skill_in_txn(&tx, skill_name, rec_id, skill_path, installed_at, status)?;
        tx.commit()?;
        Ok(())
    } else {
        upsert_skill_in_txn(conn, skill_name, rec_id, skill_path, installed_at, status)
    }
}

/// The body of [`upsert_skill`], run inside whichever transaction the caller holds.
fn upsert_skill_in_txn(
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

/// Indexes one durable external per-step judgment.
pub fn index_step_judgment(conn: &Connection, event: &StepJudgmentEvent) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO step_judgments(
            event_id, rec_id, seq, verdict, rationale, suggested_action, judge, task_key, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            event.event_id,
            event.rec_id,
            event.seq as i64,
            event.verdict.as_str(),
            event.rationale,
            event.suggested_action,
            event.judge,
            event.task_key,
            event.created_at,
        ],
    )?;
    Ok(())
}

/// Indexes one durable skill regression base case.
pub fn index_regression_base_case(
    conn: &Connection,
    event: &RegressionBaseCaseEvent,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO regression_base_cases(
            event_id, skill_name, skill_hash, rec_id, case_name, reference_version, notes, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            event.event_id,
            event.skill_name,
            event.skill_hash,
            event.rec_id,
            event.case_name,
            event.reference_version,
            event.notes,
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

/// Keeps the live catalog current for appended per-step judgments.
pub fn sync_step_judgments(events: &[StepJudgmentEvent]) -> Result<()> {
    let conn = open()?;
    let tx = conn.unchecked_transaction()?;
    for event in events {
        index_step_judgment(&tx, event)?;
    }
    tx.commit()?;
    Ok(())
}

/// Keeps the live catalog current for an appended regression base case.
pub fn sync_regression_base_case(event: &RegressionBaseCaseEvent) -> Result<()> {
    let conn = open()?;
    index_regression_base_case(&conn, event)
}
