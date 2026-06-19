//! galdr — local Record & Replay for agent skills.
//!
//! Records the tool calls a harness already emits (not pixels), stores them as an
//! append-only span, and distills them into a reproducible skill. Local-first: the
//! raw lives only in `~/.galdr` and nothing leaves the machine.

mod distill;
mod ext;
mod hook;
mod paths;
mod record;
mod span;
mod summary;

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
    /// Without `--from`, generate the draft (scaffolding). With `--from <file>`,
    /// install as the final skill the distillation the agent prepared in that file.
    Distill {
        /// rec_id of the recording to distill.
        id: String,
        /// Install the final SKILL.md from this file (distilled by the agent).
        #[arg(long, value_name = "FILE")]
        from: Option<PathBuf>,
    },

    /// List closed recordings.
    List,
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
        Commands::Distill { id, from } => exit_on_error(distill::distill(&id, from.as_deref())),
        Commands::List => exit_on_error(record::list()),
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
