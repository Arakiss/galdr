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
mod engine;
mod ext;
mod hook;
mod ipc;
mod parametrize;
mod paths;
mod record;
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
    List,

    /// Show one recording with its steps.
    Show {
        /// rec_id of the recording.
        id: String,
    },

    /// List installed skills and their provenance.
    Skills,

    /// Open the terminal UI to browse recordings, inspect spans, and audit skills.
    Tui,

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
        Commands::List => exit_on_error(cmd_list()),
        Commands::Show { id } => exit_on_error(cmd_show(&id)),
        Commands::Skills => exit_on_error(cmd_skills()),
        Commands::Tui => exit_on_error(tui::run()),
        Commands::Diff { a, b } => exit_on_error(cmd_diff(&a, &b)),
        Commands::Parametrize { a, b, emit } => {
            exit_on_error(parametrize::parametrize(&a, &b, emit))
        }
        Commands::Reindex => exit_on_error(cmd_reindex()),
        Commands::Daemon { detach } => exit_on_error(daemon::run(detach)),
    }
}

/// Resolves catalog reads through three tiers, newest data first: the live daemon,
/// then the read-only database, then an in-memory index built straight from disk.
/// Whichever answers first wins; the disk tiers guarantee the CLI keeps working
/// even with no daemon and no usable database file.
fn cmd_list() -> anyhow::Result<()> {
    let recordings = if let Ok(ipc::Response::Recordings { recordings }) =
        ipc::query(&ipc::Request::ListRecordings)
    {
        recordings
    } else if let Some(rows) = from_db(catalog::list_recordings) {
        rows
    } else {
        // Last resort: never let `list` regress, even if SQLite is unusable.
        return record::list();
    };
    print_recordings(&recordings);
    Ok(())
}

fn cmd_show(id: &str) -> anyhow::Result<()> {
    let detail = if let Ok(ipc::Response::Recording { recording }) =
        ipc::query(&ipc::Request::ShowRecording { id: id.to_string() })
    {
        recording
    } else {
        from_db(|c| catalog::show_recording(c, id)).flatten()
    };
    match detail {
        Some(detail) => print_recording_detail(&detail),
        None => println!("recording {id} not found"),
    }
    Ok(())
}

fn cmd_skills() -> anyhow::Result<()> {
    let skills = if let Ok(ipc::Response::Skills { skills }) = ipc::query(&ipc::Request::ListSkills)
    {
        skills
    } else {
        from_db(catalog::list_skills).unwrap_or_default()
    };
    print_skills(&skills);
    Ok(())
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
        "catalog rebuilt: {} recordings, {} steps, {} skills",
        stats.recordings, stats.steps, stats.skills
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
    for skill in skills {
        let provenance = match &skill.rec_id {
            Some(id) if skill.orphan => format!("← {id} (orphan)"),
            Some(id) => format!("← {id}"),
            None => "← (no provenance)".to_string(),
        };
        println!("{:<28}  {}", skill.skill_name, provenance);
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
