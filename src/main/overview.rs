/// Print the error and exit with code 1. Only for interactive subcommands: never
/// used on the sensor path.
fn exit_on_error(result: anyhow::Result<()>) {
    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

/// Resolves a recording reference (a rec_id, a unique prefix, a name, or `None` for the
/// most recent) to a rec_id — or prints a friendly error and exits. Lets every command
/// that takes a recording accept a human-typed reference instead of a 26-char ULID.
fn resolve_or_exit(reference: Option<&str>) -> String {
    match record::resolve_ref(reference) {
        Ok(id) => id,
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}

/// Like [`resolve_or_exit`] for evaluation events: prefers the active recording, so an
/// outcome labelled while a recording is live lands on that recording, not on the last
/// closed one.
fn resolve_event_or_exit(reference: Option<&str>) -> String {
    match record::resolve_event_ref(reference) {
        Ok(id) => id,
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}

/// `galdr` with no subcommand: a friendly home screen for a human — where you are
/// (active recording, what you've collected) and where to go next. Styled for a
/// terminal, plain for a pipe.
fn cmd_overview() -> anyhow::Result<()> {
    println!(
        "{} {}  {}",
        style::accent("galdr"),
        env!("CARGO_PKG_VERSION"),
        style::dim("— Record & Replay for agent skills")
    );
    println!();

    match record::read_active_all().as_slice() {
        [] => println!("  {}", style::dim("no active recording")),
        [active] => println!(
            "  {} recording {}  {}",
            style::red("●"),
            style::bold(&format!("\"{}\"", active.name)),
            style::dim("— galdr rec stop when done")
        ),
        many => println!(
            "  {} {} recordings active  {}",
            style::red("●"),
            style::bold(&many.len().to_string()),
            style::dim("— galdr rec status")
        ),
    }
    let recordings = record::all_recordings().len();
    let (skills, from_galdr) = skill_counts();
    println!("  {recordings} recordings · {skills} skills ({from_galdr} from galdr)");
    println!();

    println!("  {}", style::bold("next"));
    let step = |cmd: &str, desc: &str| {
        println!(
            "    {}  {}",
            style::accent(&format!("{cmd:<24}")),
            style::dim(desc)
        );
    };
    step("galdr rec start <name>", "record a task your agent does");
    step("galdr distill", "turn the last recording into a skill");
    step("galdr suggest", "repeated tasks worth a skill");
    step("galdr bench", "how reliably your skills replay");
    step("galdr tui", "browse recordings, spans, and skills");
    println!();
    println!(
        "  {}",
        style::dim("galdr <command> --help · galdr --help for everything")
    );
    Ok(())
}

/// `(total skills, skills distilled by galdr)`, best-effort — zeros if the catalog can't
/// be read, since the overview must never fail.
fn skill_counts() -> (usize, usize) {
    let Ok(conn) = catalog::open_in_memory_indexed() else {
        return (0, 0);
    };
    let Ok(skills) = catalog::list_skills(&conn) else {
        return (0, 0);
    };
    let from_galdr = skills
        .iter()
        .filter(|s| s.origin == catalog::ORIGIN_GALDR)
        .count();
    (skills.len(), from_galdr)
}
