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
        "SELECT seq, tool_name, COALESCE(event_kind,'tool_call'), COALESCE(ts,''), COALESCE(summary,'')
         FROM steps WHERE rec_id = ?1 ORDER BY seq",
    )?;
    let mut steps = stmt
        .query_map(params![id], |r| {
            Ok(StepRow {
                seq: r.get(0)?,
                tool_name: r.get(1)?,
                event_kind: r.get(2)?,
                ts: r.get(3)?,
                summary: r.get(4)?,
                judgments: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    attach_step_judgments(conn, id, &mut steps)?;
    Ok(Some(RecordingDetail { recording, steps }))
}

fn attach_step_judgments(conn: &Connection, rec_id: &str, steps: &mut [StepRow]) -> Result<()> {
    let mut by_seq = std::collections::BTreeMap::<i64, Vec<StepJudgmentRow>>::new();
    let mut stmt = conn.prepare(
        "SELECT seq, event_id, rec_id, verdict, rationale, suggested_action, judge, task_key, created_at
         FROM step_judgments WHERE rec_id = ?1 ORDER BY seq, created_at, event_id",
    )?;
    let rows = stmt.query_map(params![rec_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            StepJudgmentRow {
                event_id: r.get(1)?,
                rec_id: r.get(2)?,
                seq: r.get(0)?,
                verdict: r.get(3)?,
                rationale: r.get(4)?,
                suggested_action: r.get(5)?,
                judge: r.get(6)?,
                task_key: r.get(7)?,
                created_at: r.get(8)?,
            },
        ))
    })?;
    for row in rows {
        let (seq, judgment) = row?;
        by_seq.entry(seq).or_default().push(judgment);
    }
    for step in steps {
        step.judgments = by_seq.remove(&step.seq).unwrap_or_default();
    }
    Ok(())
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
            origin: default_skill_origin(),
        })
    })?;
    let mut skills = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    // Classify origin from the file on disk (the catalog stays a pure index).
    for skill in &mut skills {
        skill.origin = skill_origin(&skill.skill_path).to_string();
    }
    Ok(skills)
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

/// Lists external per-step judgments, newest first.
pub fn list_step_judgments(
    conn: &Connection,
    rec_id: Option<&str>,
) -> Result<Vec<StepJudgmentRow>> {
    let map = |r: &rusqlite::Row<'_>| {
        Ok(StepJudgmentRow {
            event_id: r.get(0)?,
            rec_id: r.get(1)?,
            seq: r.get(2)?,
            verdict: r.get(3)?,
            rationale: r.get(4)?,
            suggested_action: r.get(5)?,
            judge: r.get(6)?,
            task_key: r.get(7)?,
            created_at: r.get(8)?,
        })
    };

    if let Some(rec_id) = rec_id {
        let mut stmt = conn.prepare(
            "SELECT event_id, rec_id, seq, verdict, rationale, suggested_action, judge, task_key, created_at
             FROM step_judgments
             WHERE rec_id = ?1
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map(params![rec_id], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        let mut stmt = conn.prepare(
            "SELECT event_id, rec_id, seq, verdict, rationale, suggested_action, judge, task_key, created_at
             FROM step_judgments
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map([], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Lists skill regression base cases, newest first.
pub fn list_regression_base_cases(
    conn: &Connection,
    skill_name: Option<&str>,
) -> Result<Vec<RegressionBaseCaseRow>> {
    let map = |r: &rusqlite::Row<'_>| {
        Ok(RegressionBaseCaseRow {
            event_id: r.get(0)?,
            skill_name: r.get(1)?,
            skill_hash: r.get(2)?,
            rec_id: r.get(3)?,
            case_name: r.get(4)?,
            reference_version: r.get(5)?,
            notes: r.get(6)?,
            created_at: r.get(7)?,
        })
    };

    if let Some(skill_name) = skill_name {
        let mut stmt = conn.prepare(
            "SELECT event_id, skill_name, skill_hash, rec_id, case_name, reference_version, notes, created_at
             FROM regression_base_cases
             WHERE skill_name = ?1
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map(params![skill_name], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        let mut stmt = conn.prepare(
            "SELECT event_id, skill_name, skill_hash, rec_id, case_name, reference_version, notes, created_at
             FROM regression_base_cases
             ORDER BY created_at DESC, event_id DESC",
        )?;
        let rows = stmt.query_map([], map)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
