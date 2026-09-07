fn cmd_evaluations(skill: Option<&str>, json: bool) -> anyhow::Result<()> {
    let evaluations =
        from_db(|conn| catalog::list_skill_evaluations(conn, skill)).unwrap_or_default();
    if json {
        return print_json(&evaluations);
    }
    print_evaluations(&evaluations);
    Ok(())
}

fn cmd_outcome(action: OutcomeAction) -> anyhow::Result<()> {
    match action {
        OutcomeAction::Usage {
            skill,
            rec_id,
            task_kind,
            outcome,
            retries,
            manual_interventions,
            notes,
        } => {
            let rec_id = resolve_event_or_exit(rec_id.as_deref());
            let event = outcome::record_usage(outcome::UsageInput {
                skill_name: skill,
                rec_id,
                task_kind,
                outcome,
                retries,
                manual_intervention_count: manual_interventions,
                notes,
            })?;
            warn_if_skill_missing(&event.skill_name);
            println!(
                "usage recorded: {} {} outcome={} rec_id={}",
                event.event_id, event.skill_name, event.outcome, event.rec_id
            );
        }
        OutcomeAction::Label {
            skill,
            rec_id,
            evaluator,
            label,
            confidence,
            notes,
        } => {
            let event = outcome::record_outcome(outcome::OutcomeInput {
                skill_name: skill,
                rec_id,
                evaluator_kind: evaluator,
                label,
                confidence,
                notes,
            })?;
            warn_if_skill_missing(&event.skill_name);
            println!(
                "outcome recorded: {} {} {}:{} confidence={:.2}",
                event.event_id,
                event.skill_name,
                event.evaluator_kind,
                event.label,
                event.confidence
            );
        }
        OutcomeAction::List { skill, json } => {
            let usages = from_db(|conn| catalog::list_skill_usage(conn, skill.as_deref()))
                .unwrap_or_default();
            let outcomes = from_db(|conn| catalog::list_skill_outcomes(conn, skill.as_deref()))
                .unwrap_or_default();
            if json {
                return print_json(&serde_json::json!({
                    "usage": usages,
                    "labels": outcomes,
                }));
            }
            print_usage(&usages);
            print_outcomes(&outcomes);
        }
    }
    Ok(())
}

fn cmd_judge(action: JudgeAction) -> anyhow::Result<()> {
    match action {
        JudgeAction::Import {
            reference,
            file,
            evaluator,
            json,
        } => {
            let raw = read_stdin_or_file(file.as_deref())?;
            let raw = prepare_judgment_import(&raw, reference.as_deref(), evaluator.as_deref())?;
            let report = judge::import_json(&raw)?;
            if json {
                return print_json(&report);
            }
            println!(
                "judgments imported: {} judgment(s) across {} recording(s)",
                report.imported, report.recordings
            );
            if !report.tasks.is_empty() {
                println!("  task_key(s): {}", report.tasks.join(", "));
            }
        }
        JudgeAction::Summary {
            reference,
            task,
            json,
        } => {
            let summary = judge::summarize(task.as_deref(), reference.as_deref())?;
            if json {
                return print_json(&summary);
            }
            print!("{}", judge::render_summary(&summary));
        }
    }
    Ok(())
}

fn cmd_regress(action: RegressAction) -> anyhow::Result<()> {
    match action {
        RegressAction::Pin {
            skill,
            rec_id,
            case_label,
            notes,
        } => {
            let rec_id = record::resolve_event_ref(Some(&rec_id))?;
            let event = regression::record_base_case(regression::BaseCaseInput {
                skill_name: skill,
                rec_id,
                case_name: Some(case_label),
                reference_version: None,
                notes,
            })?;
            println!(
                "base case pinned: {} {} rec_id={} hash={}",
                event.event_id,
                event.skill_name,
                event.rec_id,
                event.skill_hash.as_deref().unwrap_or("unknown")
            );
            println!(
                "  galdr records the guardrail; rerun this case after skill edits and record outcomes."
            );
        }
        RegressAction::Status { skill, json } => {
            let status = regression::status(&skill);
            if json {
                return print_json(&status);
            }
            print!("{}", regression::render_status(&status));
        }
    }
    Ok(())
}

fn read_stdin_or_file(file: Option<&Path>) -> anyhow::Result<String> {
    use anyhow::Context as _;

    if let Some(file) = file {
        return std::fs::read_to_string(file)
            .with_context(|| format!("could not read {}", file.display()));
    }
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    Ok(raw)
}

fn prepare_judgment_import(
    raw: &str,
    reference: Option<&str>,
    evaluator: Option<&str>,
) -> anyhow::Result<String> {
    let Some(reference) = reference else {
        if evaluator.is_some() {
            anyhow::bail!("--evaluator requires a recording reference");
        }
        return Ok(raw.to_string());
    };
    let rec_id = record::resolve_ref(Some(reference))?;
    let value: serde_json::Value = serde_json::from_str(raw)?;
    match value {
        serde_json::Value::Array(judgments) => Ok(serde_json::json!({
            "rec_id": rec_id,
            "judge": evaluator,
            "judgments": judgments,
        })
        .to_string()),
        serde_json::Value::Object(mut map) if map.contains_key("judgments") => {
            map.entry("rec_id".to_string())
                .or_insert_with(|| serde_json::Value::String(rec_id));
            if let Some(evaluator) = evaluator {
                map.entry("judge".to_string())
                    .or_insert_with(|| serde_json::Value::String(evaluator.to_string()));
            }
            Ok(serde_json::Value::Object(map).to_string())
        }
        _ => anyhow::bail!(
            "with a recording reference, judgment input must be an array or an object with `judgments`"
        ),
    }
}

/// Warns (without failing) when an outcome is recorded for a skill that is not
/// installed. The event is still written — a skill can be uninstalled after use —
/// but a typo'd name would otherwise silently poison the supervised-data lane.
fn warn_if_skill_missing(skill_name: &str) {
    if !outcome::skill_exists(skill_name) {
        eprintln!(
            "warning: skill '{skill_name}' is not installed under {}; recording it anyway",
            paths::skills_root()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "the skills root".to_string())
        );
    }
}
