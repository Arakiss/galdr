
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

    /// Ingest and summarize external per-step judgments for on-policy distillation.
    Judge {
        #[command(subcommand)]
        action: JudgeAction,
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

    /// Pin and inspect regression base cases for edited skills.
    Regress {
        #[command(subcommand)]
        action: RegressAction,
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
    Doctor,

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

#[derive(Subcommand)]
enum JudgeAction {
    /// Import external per-step judgments from JSON on stdin or a file.
    ///
    /// Input can be one recording object, an array of recording objects, an envelope
    /// with `recordings`, or (when a reference is passed) an array of judgments. Each
    /// judgment uses either `seq` (0-based span index) or `step` (1-based display
    /// index), plus verdict `ok` or `fork`, a rationale, and optional
    /// `suggested_action`.
    Import {
        /// Optional recording to attach a bare judgment array/object to.
        reference: Option<String>,
        /// Read JSON from this file instead of stdin.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Evaluator source, e.g. human, llm_review, strong_judge. Requires a recording reference.
        #[arg(long)]
        evaluator: Option<String>,
        /// Emit machine-readable JSON instead of a short report.
        #[arg(long)]
        json: bool,
    },
    /// Summarize measured fork points across attempts for one task.
    Summary {
        /// Recording reference whose task name should be summarized. Omit with
        /// `--task` to summarize a task name explicitly.
        reference: Option<String>,
        /// Recording/task name to summarize across attempts.
        #[arg(long)]
        task: Option<String>,
        /// Emit machine-readable JSON instead of a report.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum RegressAction {
    /// Pin one recording as a regression base case for the current skill version.
    Pin {
        /// Skill name under the skills root.
        #[arg(long)]
        skill: String,
        /// Recording that represents the base case.
        #[arg(long = "rec")]
        rec_id: String,
        /// Human-readable base-case label.
        #[arg(long = "case")]
        case_label: String,
        /// Optional operator note.
        #[arg(long)]
        notes: Option<String>,
    },
    /// Show pinned base cases and whether the skill changed since the latest pin.
    Status {
        /// Skill name under the skills root.
        #[arg(long)]
        skill: String,
        /// Emit machine-readable JSON instead of a report.
        #[arg(long)]
        json: bool,
    },
}
