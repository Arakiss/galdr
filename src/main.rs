//! galdr — local Record & Replay for agent skills.
//!
//! Records the tool calls a harness already emits (not pixels), stores them as an
//! append-only span, and distills them into a reproducible skill. Local-first: the
//! raw lives only in `~/.galdr` and nothing leaves the machine.

mod catalog;
mod config;
mod daemon;
mod diff;
mod distill;
mod doctor;
mod engine;
mod export;
mod ext;
mod harness;
mod hook;
mod ipc;
mod link;
mod outcome;
mod parametrize;
mod paths;
mod record;
mod setup;
mod span;
mod summary;
mod tui;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "galdr",
    version,
    about = "Local Record & Replay for agent skills"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// PostToolUse sensor: read the event from stdin and record it if a session is
    /// active. Meant to be invoked from a hook. Always exits with code 0.
    Hook,

    /// Control a recording.
    Rec {
        #[command(subcommand)]
        action: RecAction,
    },

    /// Distill a recording into a skill.
    ///
    /// Without flags, generate the draft (scaffolding). With `--from <file>`,
    /// install the distillation the agent prepared. With `--auto`, a local MLX
    /// engine writes the finished skill (falling back to the draft if unavailable).
    Distill {
        /// rec_id of the recording to distill.
        id: String,
        /// Install the final SKILL.md from this file (distilled by the agent).
        #[arg(long, value_name = "FILE", conflicts_with = "auto")]
        from: Option<PathBuf>,
        /// Distill autonomously with a local MLX engine.
        #[arg(long)]
        auto: bool,
        /// Engine for `--auto`: mlx-http, mlx-subprocess, or agent.
        #[arg(long, value_name = "ENGINE", requires = "auto")]
        engine: Option<String>,
    },

    /// List closed recordings.
    List {
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Show one recording with its steps.
    Show {
        /// rec_id of the recording.
        id: String,
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// List installed skills and their provenance.
    Skills {
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Detect the agent harnesses installed on this system.
    Harnesses {
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Make distilled skills discoverable by every installed harness.
    ///
    /// galdr installs a skill once in the open-standard root; this links it into
    /// each detected harness's skills directory (Claude Code, Codex, Cursor) so the
    /// harness it was recorded in can actually load it.
    Link {
        /// Link only this skill (default: every galdr-installed skill).
        #[arg(long)]
        skill: Option<String>,
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// List skill evaluator outputs from the catalog.
    Evaluations {
        /// Limit output to one skill name.
        #[arg(long)]
        skill: Option<String>,
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Record or inspect skill usage outcomes for later offline evaluation.
    Outcome {
        #[command(subcommand)]
        action: OutcomeAction,
    },

    /// Open the terminal UI to browse recordings, inspect spans, and audit skills.
    Tui,

    /// Export a recording without raw payloads unless explicitly requested.
    Export {
        /// rec_id of the recording.
        id: String,
        /// Output directory.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        /// Include the raw span JSONL. Sensitive.
        #[arg(long)]
        include_raw: bool,
        /// Export a redacted raw copy instead of the original raw payloads.
        #[arg(long)]
        redact: bool,
    },

    /// Diagnose local galdr installation, catalog, config, and hook wiring.
    Doctor,

    /// Print or check harness setup snippets.
    Setup {
        #[command(subcommand)]
        target: SetupTarget,
    },

    /// Diff two recordings of the same task to find constants and parameters.
    Diff {
        /// rec_id of the first recording.
        a: String,
        /// rec_id of the second recording.
        b: String,
    },

    /// Parametrize two recordings into a skill whose varying inputs are named.
    Parametrize {
        /// rec_id of the first recording.
        a: String,
        /// rec_id of the second recording.
        b: String,
        /// Write the parametrized SKILL.md instead of just printing the report.
        #[arg(long)]
        emit: bool,
    },

    /// Rebuild the SQLite catalog from the spans and recordings on disk.
    Reindex,

    /// Run the supervisor daemon (catalog indexer + control socket).
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonAction>,
        /// Start in the background (detached) instead of in the foreground.
        #[arg(long)]
        detach: bool,
    },
}

#[derive(Subcommand)]
enum RecAction {
    /// Start a recording (optional name).
    Start {
        /// Human-readable name for the recording.
        name: Option<String>,
    },
    /// Stop the active recording.
    Stop,
    /// Show active recording status.
    Status,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Show whether the daemon socket is answering.
    Status,
    /// Ask the daemon to shut down gracefully.
    Stop,
}

#[derive(Subcommand)]
enum SetupTarget {
    /// Inspect or print the Claude Code PostToolUse hook snippet.
    Claude {
        /// Check whether ~/.claude/settings.json already has galdr hook wiring.
        #[arg(long)]
        check: bool,
        /// Print the recommended settings snippet.
        #[arg(long)]
        print: bool,
    },
}

#[derive(Subcommand)]
enum OutcomeAction {
    /// Record that a skill was used in a real recording.
    Usage {
        /// Skill name under ~/.agents/skills.
        #[arg(long)]
        skill: String,
        /// Recording where the skill was used.
        #[arg(long = "rec")]
        rec_id: String,
        /// Optional task family for later grouping.
        #[arg(long)]
        task_kind: Option<String>,
        /// Observed result: success, partial, failed, abandoned, unknown, etc.
        #[arg(long, default_value = outcome::OUTCOME_UNKNOWN)]
        outcome: String,
        /// Number of retries needed after using the skill.
        #[arg(long, default_value_t = 0)]
        retries: u32,
        /// Number of manual corrections/interventions needed.
        #[arg(long, default_value_t = 0)]
        manual_interventions: u32,
        /// Optional operator note.
        #[arg(long)]
        notes: Option<String>,
    },
    /// Record an explicit label or review for a skill.
    Label {
        /// Skill name under ~/.agents/skills.
        #[arg(long)]
        skill: String,
        /// Optional recording used as evidence.
        #[arg(long = "rec")]
        rec_id: Option<String>,
        /// Evaluator source: human, llm_review, test, session_outcome, etc.
        #[arg(long, default_value = "human")]
        evaluator: String,
        /// Label: accepted, rejected, regression, useful, needs_review, etc.
        #[arg(long)]
        label: String,
        /// Evaluator confidence, from 0 to 1.
        #[arg(long, default_value_t = 1.0)]
        confidence: f64,
        /// Optional operator note.
        #[arg(long)]
        notes: Option<String>,
    },
    /// List captured usage and outcome labels.
    List {
        /// Limit output to one skill name.
        #[arg(long)]
        skill: Option<String>,
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Hook => {
            // The sensor must NEVER propagate a failure to the agent session. We
            // silence the panic message, catch any unwind, and always exit 0, no
            // matter what happens inside `hook::run`.
            std::panic::set_hook(Box::new(|_| {}));
            let _ = std::panic::catch_unwind(|| {
                let _ = hook::run();
            });
            std::process::exit(0);
        }
        Commands::Rec { action } => {
            let result = match action {
                RecAction::Start { name } => record::start(name),
                RecAction::Stop => record::stop(),
                RecAction::Status => cmd_rec_status(),
            };
            exit_on_error(result);
        }
        Commands::Distill {
            id,
            from,
            auto,
            engine,
        } => {
            if auto {
                exit_on_error(distill::distill_auto(&id, engine.as_deref()))
            } else {
                exit_on_error(distill::distill(&id, from.as_deref()))
            }
        }
        Commands::List { json } => exit_on_error(cmd_list(json)),
        Commands::Show { id, json } => exit_on_error(cmd_show(&id, json)),
        Commands::Skills { json } => exit_on_error(cmd_skills(json)),
        Commands::Harnesses { json } => exit_on_error(cmd_harnesses(json)),
        Commands::Link { skill, json } => exit_on_error(cmd_link(skill.as_deref(), json)),
        Commands::Evaluations { skill, json } => {
            exit_on_error(cmd_evaluations(skill.as_deref(), json))
        }
        Commands::Outcome { action } => exit_on_error(cmd_outcome(action)),
        Commands::Tui => exit_on_error(tui::run()),
        Commands::Export {
            id,
            out,
            include_raw,
            redact,
        } => exit_on_error(export::export_recording(&id, &out, include_raw, redact)),
        Commands::Doctor => exit_on_error(doctor::run()),
        Commands::Setup { target } => match target {
            SetupTarget::Claude { check, print } => {
                if print {
                    setup::claude_print();
                }
                if check || !print {
                    exit_on_error(setup::claude_check());
                }
            }
        },
        Commands::Diff { a, b } => exit_on_error(cmd_diff(&a, &b)),
        Commands::Parametrize { a, b, emit } => {
            exit_on_error(parametrize::parametrize(&a, &b, emit))
        }
        Commands::Reindex => exit_on_error(cmd_reindex()),
        Commands::Daemon { action, detach } => match action {
            Some(DaemonAction::Status) => exit_on_error(cmd_daemon_status()),
            Some(DaemonAction::Stop) => exit_on_error(cmd_daemon_stop()),
            None => exit_on_error(daemon::run(detach)),
        },
    }
}

fn cmd_rec_status() -> anyhow::Result<()> {
    let Some(active) = record::read_active() else {
        println!("no active recording");
        return Ok(());
    };
    let span_path = paths::span_file(&active.rec_id)?;
    let steps = span::count_events(&span_path);
    println!("active recording: {}", active.name);
    println!("  rec_id: {}", active.rec_id);
    println!("  started_at: {}", active.started_at);
    println!("  steps: {steps}");
    if let Some(origin) = &active.origin_cwd {
        println!("  origin_cwd: {origin}");
    }
    match &active.bound_session {
        Some(session) => println!("  bound_session: {session}"),
        None => println!("  bound_session: (unbound — first matching event will bind)"),
    }
    println!("  span: {}", span_path.display());
    if let Some(transcript_path) = active.transcript_path {
        println!("  transcript: {transcript_path}");
    }
    Ok(())
}

fn cmd_daemon_status() -> anyhow::Result<()> {
    match ipc::query(&ipc::Request::Ping) {
        Ok(ipc::Response::Pong) => {
            println!("daemon running");
            if let Ok(pid) = std::fs::read_to_string(paths::pidfile()?) {
                println!("  pid: {}", pid.trim());
            }
        }
        _ => {
            let pidfile = paths::pidfile()?;
            if pidfile.exists() {
                println!(
                    "daemon not responding (stale pidfile: {})",
                    pidfile.display()
                );
            } else {
                println!("daemon stopped");
            }
        }
    }
    Ok(())
}

fn cmd_daemon_stop() -> anyhow::Result<()> {
    match ipc::query(&ipc::Request::Shutdown) {
        Ok(ipc::Response::Ack) => println!("daemon stopped"),
        Ok(other) => println!("daemon returned unexpected response: {other:?}"),
        Err(_) => println!("daemon is not running"),
    }
    Ok(())
}

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

fn cmd_link(skill: Option<&str>, json: bool) -> anyhow::Result<()> {
    let results = match skill {
        Some(name) => link::link_skill(name)?,
        None => link::link_all()?,
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
        "catalog rebuilt: {} recordings, {} steps, {} skills, {} usages, {} outcomes",
        stats.recordings, stats.steps, stats.skills, stats.usages, stats.outcomes
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
        let origin = if skill.origin == catalog::ORIGIN_GALDR {
            "galdr "
        } else {
            "extern"
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
            "{:<28}  {:<6}  {:<11}  readiness {:>3} ({:>3})  {:<36}  {}",
            skill.skill_name,
            origin,
            skill.status,
            skill.readiness_score,
            delta,
            provenance,
            skill.readiness_notes
        );
    }
    println!(
        "\n{} galdr · {} external",
        galdr_count,
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

/// Print the error and exit with code 1. Only for interactive subcommands: never
/// used on the sensor path.
fn exit_on_error(result: anyhow::Result<()>) {
    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
