/// A skill (or file) to run through the content gate, with the markdown to check.
struct ValidationTarget {
    label: String,
    md: String,
    draft: bool,
    skill_name: Option<String>,
}

fn cmd_validate(
    skill: Option<&str>,
    all: bool,
    file: Option<&Path>,
    strict: bool,
    json: bool,
) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let mut targets: Vec<ValidationTarget> = Vec::new();
    if let Some(path) = file {
        let md = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        // A standalone file is judged as a finished skill (drafts are an installed
        // status, not a property of an arbitrary file).
        targets.push(ValidationTarget {
            label: path.display().to_string(),
            md,
            draft: false,
            skill_name: None,
        });
    } else {
        let skills = from_db(catalog::list_skills).unwrap_or_default();
        let selected: Vec<catalog::SkillRow> = skills
            .into_iter()
            .filter(|s| match skill {
                Some(name) => s.skill_name == name,
                None if all => true,
                None => s.origin == catalog::ORIGIN_GALDR,
            })
            .collect();
        for s in selected {
            let Ok(md) = std::fs::read_to_string(&s.skill_path) else {
                continue;
            };
            let draft = matches!(
                s.status.as_str(),
                catalog::STATUS_DRAFT | catalog::STATUS_PARAM_DRAFT
            );
            let skill_name = s.skill_name.clone();
            targets.push(ValidationTarget {
                label: s.skill_name,
                md,
                draft,
                skill_name: Some(skill_name),
            });
        }
    }

    if targets.is_empty() {
        if json {
            return print_json(&serde_json::json!([]));
        }
        match skill {
            Some(name) => println!("skill '{name}' is not installed or could not be read"),
            None => {
                println!("(no skills to validate — distill one first with `galdr distill <id>`)")
            }
        }
        return Ok(());
    }

    let mut any_blocking = false;
    let mut json_items = Vec::new();
    for target in &targets {
        let ctx = validate::ValidationCtx::new(target.draft, strict);
        let report = validate::validate_skill(&target.md, &ctx);
        let blocking = report.has_blocking(strict);
        any_blocking |= blocking;

        if json {
            json_items.push(validation_target_json(target, &report, blocking));
        } else {
            print_validation_report(target, &report, blocking, strict);
        }
    }

    if json {
        print_json(&serde_json::Value::Array(json_items))?;
    }
    if any_blocking {
        anyhow::bail!("validation found blocking issue(s)");
    }
    Ok(())
}

fn validation_target_json(
    target: &ValidationTarget,
    report: &validate::ValidationReport,
    blocking: bool,
) -> serde_json::Value {
    let findings: Vec<serde_json::Value> = report
        .findings
        .iter()
        .map(|f| {
            serde_json::json!({
                "severity": severity_str(f.severity),
                "category": category_str(f.category),
                "code": f.code,
                "message": f.message,
                "line": f.line,
            })
        })
        .collect();
    let regression = target
        .skill_name
        .as_deref()
        .and_then(|skill| regression::check_skill(skill).ok());
    serde_json::json!({
        "skill": target.label,
        "draft": target.draft,
        "blocking": blocking,
        "errors": report.errors(),
        "warnings": report.warnings(),
        "findings": findings,
        "regression": regression,
    })
}

fn print_validation_report(
    target: &ValidationTarget,
    report: &validate::ValidationReport,
    blocking: bool,
    strict: bool,
) {
    let verdict = if blocking {
        "BLOCKED"
    } else if report.is_empty() {
        "clean"
    } else {
        "ok (warnings)"
    };
    let strict_note = if strict { " [strict]" } else { "" };
    println!("{} — {verdict}{strict_note}", target.label);
    print!("{report}");
    if let Some(skill_name) = target.skill_name.as_deref()
        && let Ok(status) = regression::check_skill(skill_name)
        && !status.base_cases.is_empty()
    {
        println!(
            "regression: {} base case(s) pinned{}",
            status.base_cases.len(),
            if status.changed_unverified > 0 {
                "; skill changed since at least one pin"
            } else {
                ""
            }
        );
        println!("  galdr records the guardrail only; rerun base cases after edits.");
    }
}

fn severity_str(severity: validate::Severity) -> &'static str {
    match severity {
        validate::Severity::Error => "error",
        validate::Severity::Warn => "warn",
    }
}

fn category_str(category: validate::Category) -> &'static str {
    match category {
        validate::Category::Security => "security",
        validate::Category::Optimization => "optimization",
        validate::Category::Practicality => "practicality",
    }
}
