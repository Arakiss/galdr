/// Resolves catalog reads through three tiers, newest data first: the live daemon,
/// then the read-only database, then an in-memory index built straight from disk.
/// Whichever answers first wins; the disk tiers guarantee the CLI keeps working
/// even with no daemon and no usable database file.
fn cmd_list(json: bool) -> anyhow::Result<()> {
    let recordings = if let Ok(ipc::Response::Recordings { recordings }) =
        ipc::query(&ipc::Request::ListRecordings)
    {
        recordings
    } else if let Some(rows) = from_db(catalog::list_recordings) {
        rows
    } else if json {
        Vec::new()
    } else {
        // Last resort: never let `list` regress, even if SQLite is unusable.
        return record::list();
    };
    if json {
        return print_json(&recordings);
    }
    print_recordings(&recordings);
    Ok(())
}

fn cmd_show(id: &str, json: bool) -> anyhow::Result<()> {
    let detail = if let Ok(ipc::Response::Recording { recording }) =
        ipc::query(&ipc::Request::ShowRecording { id: id.to_string() })
    {
        recording
    } else {
        from_db(|c| catalog::show_recording(c, id)).flatten()
    };
    if json {
        return print_json(&detail);
    }
    match detail {
        Some(detail) => print_recording_detail(&detail),
        None => println!("recording {id} not found"),
    }
    Ok(())
}

fn cmd_skills(json: bool) -> anyhow::Result<()> {
    let skills = if let Ok(ipc::Response::Skills { skills }) = ipc::query(&ipc::Request::ListSkills)
    {
        skills
    } else {
        from_db(catalog::list_skills).unwrap_or_default()
    };
    if json {
        return print_json(&skills);
    }
    print_skills(&skills);
    Ok(())
}

fn cmd_harnesses(json: bool) -> anyhow::Result<()> {
    let harnesses = harness::detect();
    if json {
        println!("{}", serde_json::to_string_pretty(&harnesses)?);
        return Ok(());
    }
    println!("Agent harnesses on this system:");
    for h in &harnesses {
        let mark = if h.detected { "✓" } else { " " };
        let state = if h.detected { "detected" } else { "absent" };
        let mut detail = Vec::new();
        if let Some(cfg) = &h.config_dir {
            detail.push(cfg.clone());
        }
        if h.on_path {
            detail.push("on PATH".to_string());
        }
        if !h.notes.is_empty() {
            detail.push(h.notes.clone());
        }
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!("  ({})", detail.join(" · "))
        };
        println!("{mark} {:<13} {state}{detail}", h.name);
    }
    Ok(())
}

fn cmd_link(skill: Option<&str>, all: bool, json: bool) -> anyhow::Result<()> {
    let results = match skill {
        Some(name) => link::link_skill(name)?,
        None => link::link_all(all)?,
    };
    if json {
        return print_json(&results);
    }
    if results.is_empty() {
        println!(
            "No skills to link, or no harness with a known skills directory is installed.\n\
             Distill a skill first (`galdr distill <id>`), then run `galdr link`."
        );
        return Ok(());
    }
    for r in &results {
        let mark = match r.status {
            link::LinkStatus::Linked | link::LinkStatus::AlreadyLinked => "✓",
            link::LinkStatus::SameRoot => "·",
            link::LinkStatus::Conflict | link::LinkStatus::Failed => "!",
        };
        println!(
            "{mark} {:<24} → {:<12} {}  ({})",
            r.skill,
            r.harness,
            r.status.as_str(),
            r.link_path
        );
    }
    Ok(())
}

fn cmd_setup_skill() -> anyhow::Result<()> {
    let results = skill::install()?;
    let reached: Vec<&str> = results
        .iter()
        .filter(|r| {
            !matches!(
                r.status,
                link::LinkStatus::Conflict | link::LinkStatus::Failed
            )
        })
        .map(|r| r.harness.as_str())
        .collect();
    println!(
        "galdr skill installed (version {}).",
        env!("CARGO_PKG_VERSION")
    );
    if reached.is_empty() {
        println!("No harness with a known skills directory is installed yet.");
    } else {
        println!("Discoverable in: {}", reached.join(", "));
        println!("Your agent now knows how to record → distill → replay with galdr.");
    }
    Ok(())
}
