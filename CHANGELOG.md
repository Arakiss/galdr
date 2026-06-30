# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the version is below `1.0.0`, breaking changes may land in minor releases.

## [0.11.0](https://github.com/Arakiss/galdr/compare/v0.10.0...v0.11.0) (2026-06-30)


### Features

* **tui:** lazygit-style panels with a live preview (phase 1) ([#42](https://github.com/Arakiss/galdr/issues/42)) ([9d5fad9](https://github.com/Arakiss/galdr/commit/9d5fad9d0e49b38d71ed6183277638139c1019bc))

## [0.10.0](https://github.com/Arakiss/galdr/compare/v0.9.0...v0.10.0) (2026-06-30)


### Features

* **cli:** color doctor and skills ([#40](https://github.com/Arakiss/galdr/issues/40)) ([d92ec5a](https://github.com/Arakiss/galdr/commit/d92ec5a6e31f82b482a2fab06b05a606efc4eeb9))

## [0.9.0](https://github.com/Arakiss/galdr/compare/v0.8.1...v0.9.0) (2026-06-30)


### Features

* **cli:** style list, suggest, and bench; suggest by name ([#38](https://github.com/Arakiss/galdr/issues/38)) ([61c3e73](https://github.com/Arakiss/galdr/commit/61c3e73af9de28a5b174e9d04cce8932cdd46b89))

## [0.8.1](https://github.com/Arakiss/galdr/compare/v0.8.0...v0.8.1) (2026-06-30)


### Bug fixes

* **release:** empty root component so the bot auto-tags ([#36](https://github.com/Arakiss/galdr/issues/36)) ([8e47859](https://github.com/Arakiss/galdr/commit/8e47859bbdf398215ddf93790711708c7f4fa0f5))

## [0.8.0](https://github.com/Arakiss/galdr/compare/v0.7.0...v0.8.0) (2026-06-29)


### Features

* **cli:** friendly overview screen + TTY-aware styling ([#34](https://github.com/Arakiss/galdr/issues/34)) ([2c180dd](https://github.com/Arakiss/galdr/commit/2c180dd85ab04405be10a8c69c179c7a09fb34cb))

## [0.7.0](https://github.com/Arakiss/galdr/compare/v0.6.1...v0.7.0) (2026-06-29)


### Features

* **cli:** human-friendly DX — never type a 26-char id ([#30](https://github.com/Arakiss/galdr/issues/30)) ([f300202](https://github.com/Arakiss/galdr/commit/f300202165f4737f7fceaba085b8091d1b2c633f))

## [0.6.1](https://github.com/Arakiss/galdr/compare/v0.6.0...v0.6.1) (2026-06-29)


### Documentation

* launch-ready README + demo GIF ([#27](https://github.com/Arakiss/galdr/issues/27)) ([84cd8a7](https://github.com/Arakiss/galdr/commit/84cd8a7563769b151c03be377817172e04645a8e))

## [0.6.0](https://github.com/Arakiss/galdr/compare/v0.5.1...v0.6.0) (2026-06-29)


### Features

* skill-opportunity detection (suggest) + replay-reliability benchmark (bench) ([#24](https://github.com/Arakiss/galdr/issues/24)) ([8e3a9ee](https://github.com/Arakiss/galdr/commit/8e3a9eea68b5aa5210a11cb03b770ce9b56ebe78))

## [0.5.1](https://github.com/Arakiss/galdr/compare/v0.5.0...v0.5.1) (2026-06-28)


### Bug fixes

* harden the record→distill→replay loop (control-cmd noise, diff filtering, keyed secrets) ([#21](https://github.com/Arakiss/galdr/issues/21)) ([e1325f3](https://github.com/Arakiss/galdr/commit/e1325f348cbcbe5a2200ce20426def1282044e8b))

## [0.5.0](https://github.com/Arakiss/galdr/compare/v0.4.1...v0.5.0) (2026-06-24)


### Features

* **validate:** gate skill content and let the caller name skills ([#14](https://github.com/Arakiss/galdr/issues/14)) ([f38da21](https://github.com/Arakiss/galdr/commit/f38da2153b66297a11b5869763466f8f3f6d56a3))

## [0.4.1](https://github.com/Arakiss/galdr/compare/v0.4.0...v0.4.1) (2026-06-24)


### Bug fixes

* **summary:** render per-action computer-use tool calls ([#10](https://github.com/Arakiss/galdr/issues/10)) ([c0711b2](https://github.com/Arakiss/galdr/commit/c0711b2ca8d6813401de5c837a5e6db0f92d9c5d))

## [0.4.0](https://github.com/Arakiss/galdr/compare/v0.3.0...v0.4.0) (2026-06-24)


### Features

* **hook:** capture Computer Use actions, drop screenshots ([#8](https://github.com/Arakiss/galdr/issues/8)) ([bf8a9ba](https://github.com/Arakiss/galdr/commit/bf8a9ba56e5db096ca77d1d9d4a9238effd8014d))

## [0.3.0](https://github.com/Arakiss/galdr/compare/v0.2.0...v0.3.0) (2026-06-24)


### Features

* **cli:** add skill signal catalog and operator workflows ([f308651](https://github.com/Arakiss/galdr/commit/f30865187270fd76d76fbad030b89fc7d7ee4764))
* multi-harness record and replay, finished-in-one, self-skill ([a51862d](https://github.com/Arakiss/galdr/commit/a51862d9c5495840a4ff222c396af84333e4cf1b))


### Bug fixes

* **doctor:** treat legacy skill provenance as a warning ([c7683db](https://github.com/Arakiss/galdr/commit/c7683db28de1935849ee60b32647b6dee98047cf))
* recognize guarded Claude hook wiring ([2157fb4](https://github.com/Arakiss/galdr/commit/2157fb4f17da6af8b816c52e70bd827b7d023c33))
* **security:** harden sensor, redaction, loopback, paths, and daemon ([7ff4a21](https://github.com/Arakiss/galdr/commit/7ff4a213fa2f49733cfea39450a75e85d66188c4))
* **setup:** detect absolute galdr hook commands ([455c2c1](https://github.com/Arakiss/galdr/commit/455c2c1eb679a8bd0f8c49287eac57aafa7c6ccb))


### Documentation

* add code of conduct, expand contributing, add CI badge ([48a75a8](https://github.com/Arakiss/galdr/commit/48a75a878d988b699088dfe6b88f20c03b01f868))
* document readiness metrics and outcome capture ([3e520a7](https://github.com/Arakiss/galdr/commit/3e520a7d17390020539a04b3f8caa0d2ff94dcb1))

## [Unreleased]

### Added

- **Finished in one.** `galdr distill <id>` (no flags) now renders a *complete*, valid
  skill straight from the span — in the open-standard anatomy (`When to use` / `Inputs`
  / `Steps` / `Verification`, the same shape Codex Record & Replay uses) — and installs
  it, no agent pass required. `--draft` keeps the agent-assisted scaffolding for a higher
  ceiling; `--auto` (local MLX) now falls back to this complete skill instead of a draft.
- `galdr setup codex` (`--check` / `--print`) wires galdr's sensor into Codex's
  `~/.codex/hooks.json`, which shares Claude Code's hook shape. `galdr harnesses` now
  reports the Codex sensor status alongside Claude Code's.
- `galdr setup skill` installs galdr's *own* skill — a `SKILL.md` that teaches an agent
  how to drive galdr (record → distill → replay) — into every installed harness. The
  skill is embedded in the binary and version-stamped, so it never drifts from the CLI;
  `galdr doctor` flags a stale or missing one. galdr dogfooding its own thesis.
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
- TUI redesign: a tabbed layout (Overview / Recordings / Skills / Harnesses) with a
  number-key and tab-key switcher. The Overview is a dashboard — stat cards, a system
  harness panel, and recent activity. A dedicated Skills tab separates galdr-distilled
  skills from external ones and color-codes readiness; a Harnesses tab shows which
  agent harnesses are installed and whether galdr's sensor is wired into each.
- `galdr harnesses` (with `--json`) detects the agent harnesses installed on the
  system (Claude Code, Codex, Cursor, Gemini CLI, Aider, Windsurf) by config dir and
  `PATH`, and reports whether galdr's hook is wired in.
- `galdr skills` now labels each skill `galdr` or `external` and lists galdr-distilled
  skills first, so your own distilled skills are not buried among other harnesses'.
- `--json` on every read command (`list`, `show`, `skills`, `evaluations`,
  `harnesses`, `outcome list`): the CLI is the AI-first surface, so an agent consumes
  galdr's state as structured data instead of scraping a human table.

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

### Security

Pre-release hardening pass (adversarial review: a local multi-agent audit plus an
independent Codex review; both reviews agreed on the core issues):

- **Sensor DoS** — the hook now caps stdin at 16 MiB, so a hostile/buggy harness can no
  longer OOM-abort the process before the panic guard runs (the "never break the session"
  contract).
- **Private data exposure** — `~/.galdr` (spans hold raw tool data) is locked to `0700`,
  so another local user cannot read your recordings or catalog. This also closes the
  daemon socket's bind/chmod race (the socket lives inside the now-0700 root).
- **Loopback bypass / SSRF** — `validate_loopback` parses the authority before any
  `/?#`, so `http://evil.com#@127.0.0.1` resolves to `evil.com` and is rejected; the host
  is matched by `IpAddr::is_loopback()`. The MLX HTTP client now also forbids redirects.
- **Redaction leak** — `export --redact` scrubs *every* exported file (steps.md,
  recording.json, …), not just the raw span; a Bash command summary can carry a secret too.
  The redactor gained more credential classes (JWT, GitLab, npm, Google OAuth, …) and now
  redacts whole PEM key blocks.
- **Skill prompt-injection** — untrusted recording names and recorded paths are sanitized
  (newlines collapsed, backticks neutralized, YAML escaped) before they land in the
  installed SKILL.md an agent loads; the `--auto` prompt neutralizes attempts to forge its
  untrusted-data delimiter.
- **Path traversal** — `galdr link --skill <name>` (and any raw skill name) is validated
  to a single safe path component, and a pre-existing symlinked skill directory is refused
  rather than followed.
- **Daemon hardening** — bounded request size and a read timeout on the control socket.

### Fixed

- **Distilled skills are now discoverable by the harness they were recorded in.**
  galdr installs a skill in the open-standard root (`~/.agents/skills`), but each
  harness loads from its own directory — Claude Code from `~/.claude/skills`, Codex
  from `~/.codex/skills`, Cursor from `~/.cursor/skills-cursor`. A skill that only
  lived in the open-standard root was invisible to the harness, so galdr recorded and
  distilled and then dead-ended at a file nothing loaded. Installing a skill now
  symlinks it into every detected harness's skills directory (never clobbering a real
  file of the same name), `galdr link` repairs discoverability in bulk (galdr-distilled
  skills only by default; `--all` syncs the whole open-standard root), and
  `galdr doctor` reports any galdr skill an installed harness can't see.
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
