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
            event_kind: crate::span::EventKind::ToolCall,
            human: None,
        }
    }

    fn human_click(seq: u64) -> Event {
        Event {
            ts: "2026-06-19T00:00:00Z".into(),
            seq,
            tool_name: "human.browser.click".into(),
            tool_input: serde_json::Value::Null,
            tool_response: serde_json::Value::Null,
            cwd: None,
            session_id: None,
            event_kind: crate::span::EventKind::Human,
            human: Some(crate::span::HumanEvent {
                source: crate::span::HumanSource::Browser {
                    url: Some("https://example.test/issues".into()),
                    title: Some("Issues".into()),
                    tab_id: None,
                },
                action: crate::span::HumanAction::from("human.browser.click"),
                target: Some(crate::span::HumanTarget {
                    primary: crate::span::TargetLocator::Role {
                        role: "button".into(),
                        name: Some("Create issue".into()),
                    },
                    alternates: Vec::new(),
                    role: Some("button".into()),
                    name: Some("Create issue".into()),
                    text: None,
                    label: None,
                    placeholder: None,
                    element_summary: None,
                }),
                value: None,
                verification_hint: Some("Confirm the issue exists.".into()),
                frame_ref: None,
            }),
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
            closed_reason: None,
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
        assert_eq!(version, 7);
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
    fn migrate_adds_event_kind_to_v4_steps() {
        let conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn, false).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE recordings (
                rec_id TEXT PRIMARY KEY,
                name TEXT,
                started_at TEXT,
                ended_at TEXT,
                steps INTEGER,
                cwd TEXT
            );
            CREATE TABLE steps (
                rec_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                ts TEXT,
                summary TEXT,
                PRIMARY KEY (rec_id, seq),
                FOREIGN KEY (rec_id) REFERENCES recordings(rec_id)
            );
            CREATE TABLE skills (
                skill_name TEXT PRIMARY KEY,
                rec_id TEXT,
                skill_path TEXT,
                installed_at TEXT,
                status TEXT NOT NULL DEFAULT 'unknown',
                readiness_score INTEGER NOT NULL DEFAULT 0,
                readiness_delta INTEGER NOT NULL DEFAULT 0,
                readiness_notes TEXT NOT NULL DEFAULT ''
            );
            ",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 4_i64).unwrap();

        migrate_schema(&conn).unwrap();

        assert!(has_column(&conn, "steps", "event_kind").unwrap());
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 7);
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
        assert_eq!(detail.steps[0].event_kind, "tool_call");
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
    fn index_human_event_keeps_kind_and_semantic_summary() {
        let conn = mem();
        index_event(&conn, "01HUMAN", &human_click(0)).unwrap();
        index_recording(&conn, &recording("01HUMAN", 1)).unwrap();

        let detail = show_recording(&conn, "01HUMAN").unwrap().unwrap();
        assert_eq!(detail.steps.len(), 1);
        assert_eq!(detail.steps[0].tool_name, "human.browser.click");
        assert_eq!(detail.steps[0].event_kind, "human");
        assert_eq!(detail.steps[0].summary, "click button \"Create issue\"");
    }

    #[test]
    fn reindex_rebuilds_from_disk() {
        let root = tempfile::tempdir().unwrap();
        let spans = root.path().join("spans");
        let recordings = root.path().join("recordings");
        let judgments = root.path().join("judgments");
        let skills = root.path().join("skills");
        let outcomes = root.path().join("outcomes");
        let regression = root.path().join("regression");
        std::fs::create_dir_all(&spans).unwrap();
        std::fs::create_dir_all(&recordings).unwrap();
        std::fs::create_dir_all(&judgments).unwrap();
        std::fs::create_dir_all(skills.join("galdr-demo")).unwrap();
        std::fs::create_dir_all(&outcomes).unwrap();
        std::fs::create_dir_all(&regression).unwrap();

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
        let judgment = StepJudgmentEvent {
            event_id: "01JUDGE".into(),
            rec_id: "01AAA".into(),
            seq: 1,
            verdict: crate::judge::StepVerdict::Fork,
            rationale: "executor wrote before reading the existing file".into(),
            suggested_action: Some("read the file first".into()),
            judge: Some("external".into()),
            task_key: "demo".into(),
            created_at: "2026-06-19T00:02:00Z".into(),
        };
        std::fs::write(
            judgments.join("01AAA.jsonl"),
            format!("{}\n", serde_json::to_string(&judgment).unwrap()),
        )
        .unwrap();

        let conn = mem();
        let stats = reindex_into_dirs(
            &conn,
            &spans,
            &recordings,
            &judgments,
            &skills,
            &outcomes,
            &regression,
        )
        .unwrap();
        assert_eq!(stats.recordings, 1);
        assert_eq!(stats.steps, 2);
        assert_eq!(stats.judgments, 1);
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
        let detail = show_recording(&conn, "01AAA").unwrap().unwrap();
        assert_eq!(detail.steps[1].judgments.len(), 1);
        assert_eq!(detail.steps[1].judgments[0].verdict, "fork");
    }
}
