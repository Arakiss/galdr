fn cmd_diff(a: &str, b: &str) -> anyhow::Result<()> {
    let report = diff::compute(a, b)?;
    print!("{}", diff::render_report(&report));
    Ok(())
}

fn cmd_reindex() -> anyhow::Result<()> {
    let stats = if let Ok(ipc::Response::Reindexed { stats }) = ipc::query(&ipc::Request::Reindex) {
        stats
    } else {
        let mut conn = catalog::open()?;
        catalog::reindex(&mut conn)?
    };
    println!(
        "catalog rebuilt: {} recordings, {} steps, {} judgments, {} skills, {} usages, {} outcomes, {} regression cases",
        stats.recordings,
        stats.steps,
        stats.judgments,
        stats.skills,
        stats.usages,
        stats.outcomes,
        stats.regression_cases
    );
    Ok(())
}

/// Runs a catalog query against the read-only database, falling back to an
/// in-memory index built from disk. Returns `None` only if neither can be opened.
fn from_db<T, F>(query: F) -> Option<T>
where
    F: Fn(&rusqlite::Connection) -> anyhow::Result<T>,
{
    if let Ok(conn) = catalog::open_readonly()
        && let Ok(value) = query(&conn)
    {
        return Some(value);
    }
    let conn = catalog::open_in_memory_indexed().ok()?;
    query(&conn).ok()
}

/// Prints any serializable value as pretty JSON on stdout. The shared sink for
/// every `--json` flag, so the AI-first surface stays consistent and parseable.
fn print_json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_recordings(recordings: &[catalog::RecordingRow]) {
    if recordings.is_empty() {
        println!("(no recordings yet — use `galdr rec start <name>`)");
        return;
    }
    for rec in recordings {
        let mark = if rec.distilled { "✓" } else { " " };
        println!(
            "{} {}  {:<20}  {} steps  {}",
            mark, rec.rec_id, rec.name, rec.steps, rec.started_at
        );
    }
}

fn print_recording_detail(detail: &catalog::RecordingDetail) {
    let rec = &detail.recording;
    println!("{}  {}", rec.rec_id, rec.name);
    println!(
        "  recorded: {} → {}",
        rec.started_at,
        rec.ended_at.as_deref().unwrap_or("(open)")
    );
    if let Some(cwd) = &rec.cwd {
        println!("  cwd: {cwd}");
    }
    println!("  distilled: {}", if rec.distilled { "yes" } else { "no" });
    println!("  steps: {}", detail.steps.len());
    for step in &detail.steps {
        println!(
            "    {:>3}. {:<10} {}",
            step.seq + 1,
            step.tool_name,
            step.summary
        );
        for judgment in &step.judgments {
            let action = judgment
                .suggested_action
                .as_ref()
                .map(|value| format!("; do instead: {value}"))
                .unwrap_or_default();
            let judge = judgment
                .judge
                .as_ref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            println!(
                "         judgment {}: {}{}{}",
                judgment.verdict, judgment.rationale, action, judge
            );
        }
    }
}

fn print_skills(skills: &[catalog::SkillRow]) {
    if skills.is_empty() {
        println!("(no skills distilled yet — use `galdr distill <id>`)");
        return;
    }
    // galdr-distilled skills first, then external ones (other harnesses / hand-authored).
    let mut sorted: Vec<&catalog::SkillRow> = skills.iter().collect();
    sorted.sort_by(|a, b| {
        let a_external = a.origin != catalog::ORIGIN_GALDR;
        let b_external = b.origin != catalog::ORIGIN_GALDR;
        a_external
            .cmp(&b_external)
            .then_with(|| a.skill_name.cmp(&b.skill_name))
    });
    let galdr_count = sorted
        .iter()
        .filter(|s| s.origin == catalog::ORIGIN_GALDR)
        .count();
    for skill in sorted {
        let is_galdr = skill.origin == catalog::ORIGIN_GALDR;
        // Pad before coloring so the ANSI codes never skew the columns.
        let name = style::accent(&format!("{:<28}", skill.skill_name));
        let origin = if is_galdr {
            style::accent("galdr ")
        } else {
            style::dim("extern")
        };
        let ready_num = format!("{:>3}", skill.readiness_score);
        let ready = if skill.readiness_score >= 80 {
            style::green(&ready_num)
        } else if skill.readiness_score >= 60 {
            style::amber(&ready_num)
        } else {
            style::red(&ready_num)
        };
        let delta = match skill.readiness_delta.cmp(&0) {
            std::cmp::Ordering::Greater => format!("+{}", skill.readiness_delta),
            std::cmp::Ordering::Less => skill.readiness_delta.to_string(),
            std::cmp::Ordering::Equal => "0".to_string(),
        };
        let provenance = match &skill.rec_id {
            Some(id) if skill.orphan => format!("← {id} (orphan)"),
            Some(id) => format!("← {id}"),
            None => "← (no provenance)".to_string(),
        };
        println!(
            "{}  {}  {:<11}  readiness {} ({:>3})  {}  {}",
            name,
            origin,
            skill.status,
            ready,
            delta,
            style::dim(&format!("{provenance:<36}")),
            style::dim(&skill.readiness_notes),
        );
    }
    println!(
        "\n{} galdr · {} external",
        style::accent(&galdr_count.to_string()),
        skills.len() - galdr_count
    );
}

fn print_evaluations(evaluations: &[catalog::SkillEvaluationRow]) {
    if evaluations.is_empty() {
        println!("(no skill evaluations yet)");
        return;
    }
    for evaluation in evaluations {
        let delta = match evaluation.score_delta.cmp(&0) {
            std::cmp::Ordering::Greater => format!("+{}", evaluation.score_delta),
            std::cmp::Ordering::Less => evaluation.score_delta.to_string(),
            std::cmp::Ordering::Equal => "0".to_string(),
        };
        println!(
            "{:<28}  {:<16}  score {:>3} ({:>3})  confidence {:.2}  {}",
            evaluation.skill_name,
            evaluation.evaluator_kind,
            evaluation.score,
            delta,
            evaluation.confidence,
            evaluation.created_at
        );
    }
}

fn print_usage(usages: &[catalog::SkillUsageRow]) {
    if usages.is_empty() {
        println!("(no skill usage events yet)");
        return;
    }
    println!("usage:");
    for usage in usages {
        println!(
            "  {:<28}  outcome {:<10} retries {:>2} interventions {:>2}  rec {}",
            usage.skill_name,
            usage.outcome,
            usage.retries,
            usage.manual_intervention_count,
            usage.rec_id
        );
    }
}

fn print_outcomes(outcomes: &[catalog::SkillOutcomeRow]) {
    if outcomes.is_empty() {
        println!("(no skill outcome labels yet)");
        return;
    }
    println!("labels:");
    for outcome in outcomes {
        let rec = outcome.rec_id.as_deref().unwrap_or("(no rec)");
        println!(
            "  {:<28}  {:<14} {:<14} confidence {:.2}  {}",
            outcome.skill_name, outcome.evaluator_kind, outcome.label, outcome.confidence, rec
        );
    }
}
