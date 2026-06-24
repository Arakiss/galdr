# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the version is below `1.0.0`, breaking changes may land in minor releases.

## [Unreleased]

### Added

- Skill catalog readiness signals: `galdr skills` now reports lifecycle status,
  readiness score, score delta, provenance, and review notes for installed skills.
- Skill evaluation history: the catalog now keeps evaluator outputs in
  `skill_evaluations`, starting with deterministic `readiness_lint` rows and leaving
  room for future human, LLM, outcome, or learned-model evaluators.
- Supervised outcome capture: `galdr outcome usage`, `galdr outcome label`, and
  `galdr outcome list` write append-only skill usage/outcome JSONL and index it for
  later offline classifier or ranker training.
- Operational commands: `galdr rec status`, `galdr daemon status`, `galdr daemon stop`,
  `galdr evaluations`, `galdr doctor`, and `galdr setup claude --check/--print`.
- Safe recording export: `galdr export <rec_id> --out <dir>` writes metadata,
  summaries, skill provenance, usage labels, and outcomes without raw payloads by
  default; `--include-raw` and `--redact` are explicit raw-export paths.
- Optional capture policy in `~/.galdr/config.json` for future recordings
  (`deny_tools`, `deny_cwd_prefixes`, `max_response_chars`).
- `GALDR_ROOT` and `GALDR_SKILLS_ROOT` environment overrides relocate the data and
  skills roots, enabling hermetic tests, throwaway profiles, and CI without hijacking
  `$HOME`. They also provide an escape hatch from the Unix-socket path-length limit.
- TUI: a substring filter (`/`) over recordings and skills, first/last (`g`/`G`) and
  page (PgUp/PgDn) navigation, a scrollable raw-payload overlay, and a live `● REC`
  indicator in the title while a recording is active.

### Changed

- Final `galdr distill <rec_id> --from <file>` installs now validate the refined skill
  has frontmatter, required sections, and no draft markers. The frontmatter check is
  structural — it requires a closing `---` and the keys inside the block, not just the
  substrings anywhere in the file.
- `galdr export --redact` now also redacts secret-shaped tokens embedded in string
  values (API keys pasted into a command or URL), not only values under a sensitive key.
- Re-distilling over an existing skill warns before overwriting, loudly when the existing
  `SKILL.md` was already a finished (refined) skill rather than a draft.
- A closed recording is `fsync`'d to disk on `galdr rec stop`, so it is durable without
  paying an `fsync` on the sensor's hot path (which stays instantaneous).
- The skill readiness delta is computed inside a transaction, so a concurrent writer can
  no longer make it stale.
- The no-daemon catalog fallback now stays current after recording closes, draft writes,
  final skill installs, and parametrized skill emits.

### Fixed

- Recordings are now scoped to the session that started them. A single global
  `active` flag meant every concurrent agent session's hook wrote into the active
  recording, so a parallel session in another project leaked its tool calls — and
  their payloads — into the span. The sensor now binds the recording to the first
  session whose event lands under the directory where `rec start` ran, and records
  only that session; `rec status` shows the `origin_cwd` and the `bound_session`.
- `galdr doctor` and `galdr setup claude --check` recognize a `galdr hook` invocation
  wrapped in a shell conditional (the resilient PATH-with-cargo-bin-fallback form),
  instead of reporting a correctly-wired hook as missing.
- `galdr daemon --detach` verifies the daemon actually answered on the control socket
  before reporting success, and fails fast with an actionable message when the socket
  path would exceed the platform's `SUN_LEN` limit.
- `galdr outcome usage|label` warns when the target skill is not installed, instead of
  silently recording it with a null skill hash and poisoning the supervised-data lane.
- Parametrized skills no longer corrupt their Markdown when a recorded value contains a
  backtick or a newline.
- The diff and parametrize "steps matched" line no longer shows `0/0` for empty recordings.
- `galdr distill --auto` raw-payload truncation no longer overshoots the configured budget.

## [0.2.0] - 2026-06-19

Phase 1 — the queryable, browsable substrate on top of the Phase 0 loop.

### Added

- **Supervisor daemon** (`galdr daemon [--detach]`): a single-instance process that
  indexes spans into the catalog over a chmod-0600 Unix socket (NDJSON IPC). It
  self-heals a corrupt/missing index on startup, reconciles dropped notifications with
  a poll-watcher, and shuts down gracefully on SIGTERM/SIGINT.
- **SQLite catalog**: a rebuildable *index, never the truth*. It stores one-line step
  summaries (no raw blobs), migrates idempotently via `PRAGMA user_version`, and
  rebuilds from `spans/` + `recordings/` (+ skill provenance) with `galdr reindex`
  (atomic temp-build-and-restore).
- **New commands**: `galdr show <id>`, `galdr skills`, `galdr reindex`. `list`/`show`/
  `skills` resolve daemon-first → read-only DB → in-memory disk scan, so the CLI works
  with or without a daemon; `list` never regresses.
- **Terminal UI** (`galdr tui`): three screens behind one `Catalog` trait — recordings
  list, span inspector (with a raw `tool_input`/`tool_response` overlay flagged as
  sensitive), and skill-provenance audit (marks orphans). A panic hook restores the
  terminal.
- **Diff-based parametrization** (`galdr diff <a> <b>`, `galdr parametrize <a> <b>
  [--emit]`): a hand-rolled global aligner separates constants from parameters across
  two runs of one task, with name inference and a High/Low confidence verdict. Low
  confidence stamps a banner and alignment notes rather than forcing a 1:1 mapping.
- **Autonomous distillation** (`galdr distill <id> --auto`): an optional local MLX
  engine writes the finished skill from the span. Loopback-only (enforced by
  `engine::validate_loopback`), the raw is wrapped in an untrusted-data delimiter,
  output is validated, and it falls back cleanly to the Phase 0 draft. Gated behind the
  `mlx` feature; configurable via `~/.galdr/config.json`.

### Changed

- The network guarantee is now stated precisely as **no external egress**: the optional
  MLX distiller talks only to loopback, enforced in code.
- The sensor now hints the daemon best-effort *after* the span append (the truth, first
  and unconditional); it never waits on or depends on the daemon.
- Minimum supported Rust version raised to 1.88 (ratatui 0.30.1).

## [0.1.0] - 2026-06-19

Phase 0 — tracer bullet. Validates that the full loop closes: record → distill → replay.

### Added

- **Sensor** (`galdr hook`): a PostToolUse hook that reads the harness event from
  stdin and appends it to the active span. It always exits 0 and catches any internal
  panic, so it can never break the agent session.
- **Recording lifecycle** (`galdr rec start [name]` / `galdr rec stop`): opens and
  closes the append-only span and writes per-recording metadata. `galdr list` shows
  closed recordings, newest first.
- **Distillation** (`galdr distill <id>`): normalizes a span into a `SKILL.md` draft
  for the agent to complete; `galdr distill <id> --from <file>` installs the agent's
  refined skill, keeping galdr the only writer of the skills directory.
- **Append-only JSONL span** as the raw, immutable source of truth, one event per
  tool call (`ts`, `seq`, `tool_name`, `tool_input`, `tool_response`, `cwd`,
  `session_id`).
- **Extension points** (`PermissionGate`, `ProvenanceSink`): two generic seams with
  neutral no-op defaults; concrete integrations plug in from outside the repository.
- Apache-2.0 license, README with a banner, and OSS hygiene docs.

[Unreleased]: https://github.com/Arakiss/galdr/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Arakiss/galdr/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Arakiss/galdr/releases/tag/v0.1.0
