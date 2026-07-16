/// Best-effort reconciliation run by the daemon's poll-watcher: pull in any events
/// that a dropped notification left unindexed. Idempotent and bounded — it only
/// touches the active recording's tail and closed recordings not yet present.
pub fn reconcile(conn: &Connection) -> Result<()> {
    // Reconcile the tail of every concurrently-active recording, not just one.
    for active in crate::record::read_active_all() {
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
        &crate::paths::judgments_dir()?,
        &crate::paths::skills_root()?,
        &crate::paths::outcomes_dir()?,
        &crate::paths::regression_dir()?,
    )
}

fn reindex_into_dirs(
    conn: &Connection,
    spans_dir: &Path,
    recordings_dir: &Path,
    judgments_dir: &Path,
    skills_root: &Path,
    outcomes_dir: &Path,
    regression_dir: &Path,
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

    // 3) External per-step judgments.
    if let Ok(entries) = std::fs::read_dir(judgments_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            for event in crate::judge::read_jsonl::<StepJudgmentEvent>(&path)? {
                index_step_judgment(&tx, &event)?;
                stats.judgments += 1;
            }
        }
    }

    // 4) Skills provenance.
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

    // 5) Skill replay outcomes.
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

    let regression_log = regression_dir.join("base_cases.jsonl");
    for event in crate::regression::read_base_cases_log(&regression_log)? {
        index_regression_base_case(&tx, &event)?;
        stats.regression_cases += 1;
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
