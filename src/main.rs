//! galdr — local Record & Replay for agent skills.
//!
//! Records the tool calls a harness already emits (not pixels), stores them as an
//! append-only span, and distills them into a reproducible skill. Local-first: the
//! raw lives only in `~/.galdr` and nothing leaves the machine.

mod bench;
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
mod launchd;
mod link;
mod observe;
mod observe_mac;
mod outcome;
mod parametrize;
mod paths;
mod record;
mod remove;
mod setup;
mod skill;
mod span;
mod style;
mod suggest;
mod summary;
mod tui;
mod upgrade;
mod validate;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "galdr",
    version,
    about = "Local Record & Replay for agent skills"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
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

    /// Record human-observation traces.
    Observe {
        #[command(subcommand)]
        action: ObserveAction,
    },

    /// Distill a recording into a skill.
    ///
    /// By default galdr writes a faithful *draft* and hands you (the agent) a brief to
    /// author the real skill, then install it with `--from`. `--fast` installs the
    /// mechanical render as a final skill in one step; `--auto` lets a local MLX engine
    /// write it (falling back to the mechanical render if the engine is unavailable).
    Distill {
        /// Which recording to distill: a rec_id, a unique id prefix, or a recording
        /// name. Omit it to distill the most recent recording (the one you just made).
        reference: Option<String>,
        /// Install the final SKILL.md from this file (the skill you authored).
        #[arg(long, value_name = "FILE", conflicts_with_all = ["auto", "fast"])]
        from: Option<PathBuf>,
        /// Skip authoring: install the faithful, mechanical render as a final skill.
        #[arg(long, conflicts_with = "auto")]
        fast: bool,
        /// Distill autonomously with a local MLX engine.
        #[arg(long)]
        auto: bool,
        /// Engine for `--auto`: mlx-http or agent.
        #[arg(long, value_name = "ENGINE", requires = "auto")]
        engine: Option<String>,
        /// Name the skill (slugified) instead of the mechanical `galdr-<slug>`.
        /// galdr supplies the mechanism; choosing a memorable, descriptive name is
        /// the caller's job — galdr does not guess one.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Refuse to install unless the skill is impeccable: optimization and
        /// documented-danger warnings also block, not just hard errors.
        #[arg(long)]
        strict: bool,
    },

    /// List closed recordings.
    List {
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Show one recording with its steps.
    Show {
        /// Which recording: a rec_id, a unique id prefix, or a name. Omit for the most
        /// recent.
        reference: Option<String>,
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
        /// Link only this skill (default: every galdr-distilled skill).
        #[arg(long)]
        skill: Option<String>,
        /// Link every skill in the open-standard root, not just galdr-distilled
        /// ones — sync the whole directory across harnesses.
        #[arg(long)]
        all: bool,
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Retire a skill: unlink it from every harness and move it aside (reversible).
    ///
    /// Removes the symlinks `galdr link` created (only those that truly point at this
    /// skill), moves the skill's directory into `~/.agents/skills/.retired/` — never a
    /// hard delete — and refreshes the catalog. Asks for confirmation at a TTY; use
    /// `--force` to skip it (required in a non-interactive context).
    Rm {
        /// The skill to retire (its directory name under the skills root).
        skill: String,
        /// Skip the interactive confirmation (required when there is no TTY).
        #[arg(long)]
        force: bool,
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
        /// Which recording: a rec_id, a unique id prefix, or a name. Omit for the most
        /// recent.
        reference: Option<String>,
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

    /// Validate installed skills (or a file) against the content gate.
    ///
    /// The same gate every `galdr distill` runs before installing: security (secrets,
    /// personal paths, dangerous commands), practicality (a real, complete skill), and
    /// optimization (a precise description, no recording noise). Exits non-zero if any
    /// blocking finding is present.
    Validate {
        /// Name of one installed skill to validate (default: galdr-distilled skills).
        skill: Option<String>,
        /// Validate every skill in the open-standard root, not just galdr's.
        #[arg(long, conflicts_with = "file")]
        all: bool,
        /// Validate an arbitrary SKILL.md file instead of an installed skill.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Treat optimization and documented-danger warnings as blocking too.
        #[arg(long)]
        strict: bool,
        /// Emit machine-readable JSON instead of a report.
        #[arg(long)]
        json: bool,
    },

    /// Diagnose local galdr installation, catalog, config, and hook wiring.
    Doctor {
        /// Emit a machine-readable health summary for an agent to self-diagnose.
        #[arg(long)]
        json: bool,
    },

    /// Check for and install a newer galdr from crates.io.
    ///
    /// The only time galdr reaches the network on its own is when you ask it to: this
    /// command (and `doctor`) query the crates.io index with a short timeout and fail
    /// soft — no connection is a note, never an error. `--check` only reports (exit 10
    /// if a newer version exists, for scripts); without it, a newer version is fetched
    /// and installed with `cargo install`, then a stale daemon is restarted.
    Upgrade {
        /// Only report whether a newer version exists; install nothing. Exit 10 when an
        /// update is available, 0 when up to date, local-ahead, or offline.
        #[arg(long)]
        check: bool,
        /// Install source: `crates` (default) or `path <dir>` to install from a local
        /// clone with `cargo install --path <dir>`.
        #[arg(long, num_args = 1..=2, value_names = ["SOURCE", "DIR"])]
        from: Option<Vec<String>>,
    },

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

    /// Measure replay reliability: per-skill hit-rate from recorded outcomes.
    ///
    /// galdr cannot run a natural-language skill (the agent does), so it cannot score
    /// a replay itself; instead it aggregates the outcomes you record with
    /// `galdr outcome usage` into a per-skill clean-replay hit-rate and an effort cost
    /// (retries, manual interventions). It is the production hit-rate a capability test
    /// cannot give. With no recorded outcomes it measures nothing and says so.
    Bench {
        /// Limit the report to one skill.
        #[arg(long)]
        skill: Option<String>,
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Suggest skill opportunities: repeated tasks not yet distilled into a skill.
    ///
    /// Signs every recording by the shape of its meaningful steps, groups the runs
    /// that share a shape, dedupes against the skills already installed, and ranks
    /// what is left by repeatability. Turns "skill opportunity" from a judgment call
    /// into a number you can query. It only sees recorded sessions.
    Suggest {
        /// Emit machine-readable JSON instead of a table.
        #[arg(long)]
        json: bool,
        /// Show only the top N opportunities.
        #[arg(long)]
        top: Option<usize>,
        /// Minimum number of recordings sharing a shape to surface it (default 2).
        #[arg(long, default_value_t = 2)]
        min_count: usize,
    },

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
    /// Start a recording (optional name). Several can run at once: each binds to the
    /// session that first acts under its directory, so parallel agents record
    /// independently. If your session already has a recording, a new one waits unbound
    /// until the first stops.
    Start {
        /// Human-readable name for the recording.
        name: Option<String>,
    },
    /// Stop an active recording. With no argument, stops the sole active recording (or
    /// errors listing them if several are active); pass a name, rec_id, or unique
    /// prefix to stop a specific one.
    Stop {
        /// Which recording to stop: a name, rec_id, or unique prefix.
        reference: Option<String>,
    },
    /// Show every active recording (name, rec_id, steps, bound session or unbound).
    Status,
}

#[derive(Subcommand)]
enum ObserveAction {
    /// Record a deterministic human-observation fixture.
    Synthetic {
        /// Human-readable name for the recording.
        name: String,
        /// Synthetic fixture to record.
        #[arg(long, value_enum, default_value_t = observe::ObserveFixture::BrowserForm)]
        fixture: observe::ObserveFixture,
    },
    /// Observe a browser workflow with a local CDP sensor and loopback collector.
    Browser {
        #[command(subcommand)]
        action: BrowserObserveAction,
    },
    /// Observe a native macOS workflow: clicks, scrolls and keystrokes captured
    /// through a listen-only event tap. macOS only; needs Input Monitoring.
    Mac {
        #[command(subcommand)]
        action: MacObserveAction,
    },
}

#[derive(Subcommand)]
enum MacObserveAction {
    /// Start a macOS-observation session (spawns the native sensor).
    Start {
        /// Human-readable name for the recording.
        name: String,
    },
    /// Stop the active macOS-observation session and write the recording.
    Stop,
    /// Show the active macOS-observation session.
    Status,
    /// Internal native sensor process.
    #[command(hide = true)]
    Serve {
        /// Recording id of the macOS-observation session.
        rec_id: String,
    },
}

#[derive(Subcommand)]
enum BrowserObserveAction {
    /// Start a browser-observation session.
    Start {
        /// Human-readable name for the recording.
        name: String,
        /// URL to open in the isolated browser profile.
        #[arg(long)]
        url: String,
        /// Chrome/Chromium-compatible browser binary to launch.
        #[arg(long, value_name = "PATH")]
        browser: Option<PathBuf>,
        /// Start only the local collector, without launching a browser.
        #[arg(long, hide = true)]
        no_open: bool,
        /// Launch the browser in headless mode. Intended for local smoke tests.
        #[arg(long, hide = true)]
        headless: bool,
    },
    /// Stop the active browser-observation session and write the recording.
    Stop,
    /// Show the active browser-observation session.
    Status,
    /// Internal loopback collector process.
    #[command(hide = true)]
    Serve {
        /// Recording id of the browser-observation session.
        rec_id: String,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Show whether the daemon socket is answering.
    Status,
    /// Ask the daemon to shut down gracefully.
    Stop,
    /// Install a macOS LaunchAgent so the daemon starts at login and restarts on crash.
    ///
    /// Writes ~/Library/LaunchAgents/dev.galdr.daemon.plist pointing at this binary,
    /// stops any loose (nohup) daemon, and loads the job with launchctl. macOS-only.
    Install,
    /// Remove the macOS LaunchAgent (unload the job and delete the plist).
    ///
    /// Leaves logs and recordings under ~/.galdr untouched. macOS-only.
    Uninstall,
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
    /// Inspect or print the Codex PostToolUse hook snippet (~/.codex/hooks.json).
    Codex {
        /// Check whether ~/.codex/hooks.json already has galdr hook wiring.
        #[arg(long)]
        check: bool,
        /// Print the recommended hooks snippet.
        #[arg(long)]
        print: bool,
    },
    /// Inspect or print the Cursor postToolUse hook snippet (~/.cursor/hooks.json).
    Cursor {
        /// Check whether ~/.cursor/hooks.json already has galdr hook wiring.
        #[arg(long)]
        check: bool,
        /// Print the recommended hooks snippet.
        #[arg(long)]
        print: bool,
    },
    /// Install galdr's own skill so every harness knows how to drive galdr.
    ///
    /// The skill is embedded in the binary and version-stamped, so it never drifts
    /// from the CLI. Installs into the open-standard root and links it into every
    /// detected harness. `--print` writes it to stdout instead.
    Skill {
        /// Print the skill to stdout instead of installing it.
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
        /// Recording where the skill was used: a rec_id, a unique prefix, or a name.
        /// Omit it to use the most recent recording.
        #[arg(long = "rec")]
        rec_id: Option<String>,
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
    let Some(command) = cli.command else {
        // `galdr` with no subcommand is a human at a terminal — show a friendly overview,
        // not a clap usage error.
        exit_on_error(cmd_overview());
        return;
    };
    match command {
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
                RecAction::Stop { reference } => record::stop(reference.as_deref()),
                RecAction::Status => cmd_rec_status(),
            };
            exit_on_error(result);
        }
        Commands::Observe { action } => match action {
            ObserveAction::Synthetic { name, fixture } => {
                exit_on_error(observe::synthetic(name, fixture))
            }
            ObserveAction::Browser { action } => match action {
                BrowserObserveAction::Start {
                    name,
                    url,
                    browser,
                    no_open,
                    headless,
                } => exit_on_error(observe::browser_start(
                    name, url, browser, no_open, headless,
                )),
                BrowserObserveAction::Stop => exit_on_error(observe::browser_stop()),
                BrowserObserveAction::Status => exit_on_error(observe::browser_status()),
                BrowserObserveAction::Serve { rec_id } => {
                    exit_on_error(observe::browser_serve(&rec_id))
                }
            },
            ObserveAction::Mac { action } => match action {
                MacObserveAction::Start { name } => exit_on_error(observe_mac::mac_start(name)),
                MacObserveAction::Stop => exit_on_error(observe_mac::mac_stop()),
                MacObserveAction::Status => exit_on_error(observe_mac::mac_status()),
                MacObserveAction::Serve { rec_id } => {
                    exit_on_error(observe_mac::mac_serve(&rec_id))
                }
            },
        },
        Commands::Distill {
            reference,
            from,
            fast,
            auto,
            engine,
            name,
            strict,
        } => {
            let id = resolve_or_exit(reference.as_deref());
            if auto {
                exit_on_error(distill::distill_auto(
                    &id,
                    engine.as_deref(),
                    strict,
                    name.as_deref(),
                ))
            } else {
                exit_on_error(distill::distill(
                    &id,
                    from.as_deref(),
                    fast,
                    strict,
                    name.as_deref(),
                ))
            }
        }
        Commands::List { json } => exit_on_error(cmd_list(json)),
        Commands::Show { reference, json } => {
            exit_on_error(cmd_show(&resolve_or_exit(reference.as_deref()), json))
        }
        Commands::Skills { json } => exit_on_error(cmd_skills(json)),
        Commands::Harnesses { json } => exit_on_error(cmd_harnesses(json)),
        Commands::Link { skill, all, json } => exit_on_error(cmd_link(skill.as_deref(), all, json)),
        Commands::Rm { skill, force } => exit_on_error(remove::run(&skill, force)),
        Commands::Evaluations { skill, json } => {
            exit_on_error(cmd_evaluations(skill.as_deref(), json))
        }
        Commands::Outcome { action } => exit_on_error(cmd_outcome(action)),
        Commands::Tui => exit_on_error(tui::run()),
        Commands::Export {
            reference,
            out,
            include_raw,
            redact,
        } => exit_on_error(export::export_recording(
            &resolve_or_exit(reference.as_deref()),
            &out,
            include_raw,
            redact,
        )),
        Commands::Validate {
            skill,
            all,
            file,
            strict,
            json,
        } => exit_on_error(cmd_validate(
            skill.as_deref(),
            all,
            file.as_deref(),
            strict,
            json,
        )),
        Commands::Doctor { json } => {
            if json {
                exit_on_error(doctor::run_json())
            } else {
                exit_on_error(doctor::run())
            }
        }
        Commands::Upgrade { check, from } => match upgrade::run(check, from) {
            Ok(code) => std::process::exit(code),
            Err(err) => {
                eprintln!("error: {err:#}");
                std::process::exit(1);
            }
        },
        Commands::Setup { target } => match target {
            SetupTarget::Claude { check, print } => {
                if print {
                    setup::claude_print();
                }
                if check || !print {
                    exit_on_error(setup::claude_check());
                }
            }
            SetupTarget::Codex { check, print } => {
                if print {
                    setup::codex_print();
                }
                if check || !print {
                    exit_on_error(setup::codex_check());
                }
            }
            SetupTarget::Cursor { check, print } => {
                if print {
                    setup::cursor_print();
                }
                if check || !print {
                    exit_on_error(setup::cursor_check());
                }
            }
            SetupTarget::Skill { print } => {
                if print {
                    print!("{}", skill::render());
                } else {
                    exit_on_error(cmd_setup_skill());
                }
            }
        },
        Commands::Diff { a, b } => exit_on_error(cmd_diff(&a, &b)),
        Commands::Parametrize { a, b, emit } => {
            exit_on_error(parametrize::parametrize(&a, &b, emit))
        }
        Commands::Bench { skill, json } => exit_on_error(bench::run(skill.as_deref(), json)),
        Commands::Suggest {
            json,
            top,
            min_count,
        } => exit_on_error(suggest::run(json, top, min_count)),
        Commands::Reindex => exit_on_error(cmd_reindex()),
        Commands::Daemon { action, detach } => match action {
            Some(DaemonAction::Status) => exit_on_error(cmd_daemon_status()),
            Some(DaemonAction::Stop) => exit_on_error(cmd_daemon_stop()),
            Some(DaemonAction::Install) => exit_on_error(launchd::install()),
            Some(DaemonAction::Uninstall) => exit_on_error(launchd::uninstall()),
            None => exit_on_error(daemon::run(detach)),
        },
    }
}

fn cmd_rec_status() -> anyhow::Result<()> {
    let actives = record::read_active_all();
    if actives.is_empty() {
        println!("no active recording");
        return Ok(());
    }
    let n = actives.len();
    println!("{n} active recording{}:", if n == 1 { "" } else { "s" });
    for active in &actives {
        let span_path = paths::span_file(&active.rec_id)?;
        let steps = span::count_events(&span_path);
        println!("  {} ({})", active.name, active.rec_id);
        println!("    started_at: {}", active.started_at);
        println!("    steps: {steps}");
        if let Some(origin) = &active.origin_cwd {
            println!("    origin_cwd: {origin}");
        }
        match &active.bound_session {
            Some(session) => println!("    bound_session: {session}"),
            None => println!(
                "    bound_session: (unbound — waiting for the first activity of a session that has no recording yet)"
            ),
        }
        println!("    span: {}", span_path.display());
        if let Some(transcript_path) = &active.transcript_path {
            println!("    transcript: {transcript_path}");
        }
    }
    Ok(())
}

fn cmd_daemon_status() -> anyhow::Result<()> {
    match ipc::query(&ipc::Request::Ping) {
        Ok(ipc::Response::Pong { version }) => {
            println!("daemon running");
            match version.as_deref() {
                Some(v) => println!("  version: {v}"),
                None => println!("  version: unknown (older daemon; restart to report it)"),
            }
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
    // Whether launchd manages it (macOS): managed, installed-but-not-loaded, or a loose
    // process. Omitted off macOS, where there is no launchd to report on.
    if let Some(line) = launchd::status_line() {
        println!("  {line}");
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
            let rec_id = resolve_or_exit(rec_id.as_deref());
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

/// A skill (or file) to run through the content gate, with the markdown to check.
struct ValidationTarget {
    label: String,
    md: String,
    draft: bool,
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
            targets.push(ValidationTarget {
                label: s.skill_name,
                md,
                draft,
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
    serde_json::json!({
        "skill": target.label,
        "draft": target.draft,
        "blocking": blocking,
        "errors": report.errors(),
        "warnings": report.warnings(),
        "findings": findings,
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
