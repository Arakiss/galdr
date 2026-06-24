<p align="center">
  <img src="assets/banner.svg" alt="galdr — Record & Replay for agent skills" width="100%">
</p>

<p align="center">
  <a href="https://github.com/Arakiss/galdr/actions/workflows/ci.yml"><img src="https://github.com/Arakiss/galdr/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0">
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust 1.88+">
  <img src="https://img.shields.io/badge/status-phase%201%20·%20catalog%20%2B%20evals%20%2B%20ops-8B7BF0.svg" alt="Status: phase 1">
  <img src="https://img.shields.io/badge/egress-none%20·%20loopback--only-4FD6C9.svg" alt="No external egress, loopback-only">
</p>

# galdr

> _galdr_ — Old Norse for a chanted spell: a sequence performed once and sung again.

**Record & Replay for agent skills.** galdr records the *tool calls* an agent harness
already emits — not pixels, not the screen — stores them as a raw, immutable span, and
distills them into a reproducible skill. Everything is local: the raw lives in
`~/.galdr` and nothing leaves the machine.

The idea: instead of re-explaining to your agent how to do a task it already did well,
**record it once** and turn it into a skill it can replay with judgment.

> [!NOTE]
> **Status: Phase 1.** Built on the proven Phase 0 loop (record → distill → replay),
> this adds the supervisor daemon, a SQLite catalog (a rebuildable *index*, never the
> truth), readiness/evaluator signals for distilled skills, operational diagnostics,
> safe export, a terminal UI, diff-based parametrization of two recordings, and optional
> autonomous distillation against a local, loopback-only MLX server. A plain
> `cargo build` needs none of the MLX path — autonomous distillation is feature-gated
> and falls back cleanly to the agent-assisted draft.

## Why tool calls, not pixels

A GUI Record & Replay records what the screen looked like. It breaks when a button
moves. An agent already emits a clean, structured trace of *what it did*: each tool
call, its input, and its result. galdr records that substrate. The replay is not a
pixel re-enactment — it is a skill the agent reads and applies with judgment.

**The honest scope.** galdr records what *your agent* did, not what *you* did by hand
outside it. Note this is narrower than it sounds: when the agent drives a browser
through a tool — a Playwright/Chrome MCP server, a browser tool — those clicks, types,
and navigations **are** tool calls, so galdr already captures and distills them like
any other step. The only thing out of scope is capturing a *human's* manual gestures
in a browser the agent never touched; that would need a separate pixel/DOM capture
layer and contradicts the "tool calls, not pixels" thesis, so it stays out of the core
(a roadmap item, and a job for the extension seam if ever). Sold honestly: this is
Record & Replay *for what your agent does* — including its web automation.

## How it works

```
agent session ──(PostToolUse)──▶ galdr hook ──append──▶ span (JSONL)
                                                              │
                                            galdr distill ◀───┘
                                                   │
                                                   ▼
                                      ~/.agents/skills/<name>/SKILL.md
```

1. **Sensor** (`galdr hook`) — invoked by the harness after each tool call. If a
   recording is active, it appends the event to the span. It is instantaneous and
   **always exits 0**: it never breaks the session, even if it fails internally.
2. **Recording** (`galdr rec start` / `stop`) — opens and closes the span, and writes
   the recording metadata.
3. **Distillation** (`galdr distill <id>`) — renders a **complete, usable** `SKILL.md`
   straight from the span, in the open-standard anatomy (`When to use` / `Inputs` /
   `Steps` / `Verification`), installs it, and links it into every installed harness.
   Finished in one, no agent pass required. For a higher ceiling, `--draft` emits
   scaffolding an agent refines, and `--auto` lets a local model write it.

## Quickstart

```sh
cargo install --path .

galdr rec start demo      # start recording
#  ... do the task with your agent (a few tool calls) ...
galdr rec stop            # close the recording, prints the rec_id

galdr list                # list recordings
galdr rec status          # inspect the active recording, if any
galdr distill <rec_id>    # → a complete, discoverable skill, in one step

# Optional, higher ceiling:
galdr distill <rec_id> --draft              # scaffolding an agent refines, then…
galdr distill <rec_id> --from <temp-file>   # …install the agent's refined skill
```

## More commands

```sh
galdr daemon --detach        # run the supervisor daemon (catalog indexer + socket)
galdr daemon status          # check whether the daemon is answering
galdr daemon stop            # ask the daemon to shut down gracefully
galdr show <rec_id>          # inspect one recording with its steps
galdr skills                 # list installed skills, galdr/external origin, provenance, and readiness
galdr harnesses              # detect agent harnesses on this system and whether galdr's sensor is wired
galdr harnesses --json       # the same, machine-readable
galdr link                   # make distilled skills discoverable by every installed harness
galdr link --skill <name>    # link just one skill
galdr evaluations            # list skill evaluator outputs from the catalog
galdr evaluations --skill <name>   # show one skill's evaluator history
galdr outcome usage --skill <name> --rec <rec_id> --outcome success
galdr outcome label --skill <name> --label accepted --evaluator human
galdr outcome list --skill <name>  # inspect captured usage/outcome labels
galdr reindex                # rebuild the SQLite catalog from disk
galdr doctor                 # diagnose config, catalog, daemon, skills, and hook wiring
galdr setup claude --check   # check Claude Code PostToolUse hook wiring
galdr setup claude --print   # print the safe settings.json snippet
galdr setup codex --check    # check Codex PostToolUse hook wiring (~/.codex/hooks.json)
galdr setup codex --print    # print the safe Codex hooks snippet
galdr tui                    # browse recordings, inspect spans, audit skills

galdr diff <a> <b>           # diff two recordings: constants vs parameters
galdr parametrize <a> <b> --emit   # write a parametrized SKILL.md (suffix -param)
galdr export <rec_id> --out ./export        # export metadata + summaries, no raw payloads
galdr export <rec_id> --out ./export --redact   # export a redacted raw copy

galdr distill <rec_id> --auto      # autonomous distillation (local MLX, see below)
```

galdr is two surfaces over one catalog: the **CLI is AI-first** and the **TUI is for
humans**. Every read command — `list`, `show`, `skills`, `evaluations`, `harnesses`,
`outcome list` — takes `--json` and emits a single parseable document, so an agent
consumes galdr without scraping a table:

```sh
galdr list --json | jq '.[].rec_id'
galdr skills --json | jq '[.[] | select(.origin == "galdr")]'
```

The daemon is optional. `list`/`show`/`skills` answer daemon-first, then from the
read-only catalog, then from a fresh in-memory index built straight from disk — so the
CLI works whether or not the daemon is running. Write paths also keep the local catalog
fresh best-effort, so a closed recording or newly installed skill is visible without
waiting for a daemon process.

`galdr doctor` checks the local root, config, daemon socket, active recording, rebuildable
catalog, skill provenance, draft skills, and Claude Code hook wiring. `galdr setup claude
--print` emits the safe hook snippet; it never mutates your settings file.

`galdr skills` is a small skill catalog, not just a name list. It reports each
skill's provenance, lifecycle status (`draft`, `final`, `param-draft`, `unknown`), a
simple 0-100 readiness score, and the latest delta versus the previously indexed
version. The score is intentionally explainable: frontmatter, required sections, draft
markers, and provenance. It is a lint/readiness guardrail, not a claim that the skill is
objectively good.

Evaluator outputs are stored separately from the skill row and can be inspected with
`galdr evaluations`. Today's built-in evaluator is `readiness_lint`; future evaluators
can add human review, LLM review, outcome evidence, or a learned model without mixing
those signals into one opaque "quality" number.

`galdr outcome` is the supervised-data lane. It records append-only local JSONL events
under `~/.galdr/outcomes/` and indexes them into the catalog:

- `galdr outcome usage` records that a skill was used in a later recording, with task
  kind, outcome, retry count, manual intervention count, and notes.
- `galdr outcome label` records explicit labels or reviews such as `accepted`,
  `rejected`, `needs_review`, `regression`, or any project-specific label.
- `galdr outcome list` shows the captured usage and labels.

This keeps learned-model training out of the agent loop while collecting the labels a
future offline classifier or ranker needs: skill content/hash, provenance, task context,
observed outcome, retries, interventions, and reviewer labels.

### Optional: autonomous distillation (local MLX)

`galdr distill <id> --auto` lets a local model write the finished skill instead of the
agent. It is **off by default** and **loopback-only** — it never reaches off the machine.

1. Build with the feature: `cargo install --path . --features mlx`.
2. Run a local OpenAI-compatible server, e.g. `mlx_lm.server` from
   [mlx-lm](https://github.com/ml-explore/mlx-lm):
   `python3 -m mlx_lm.server --model mlx-community/Qwen3-4B-Instruct-2507-4bit`
   (the default endpoint `http://127.0.0.1:8080` and model are configurable in
   `~/.galdr/config.json`).
3. `galdr distill <id> --auto` (or `--engine mlx-subprocess` to shell out to
   `python3 -m mlx_lm.generate`, which needs no `mlx` feature).

If the engine is missing or unreachable, `--auto` falls back to the Phase 0 draft and
still exits 0. Always review a machine-generated skill before relying on it.

## Wiring the sensor

For the sensor to receive events, the harness must invoke `galdr hook` in its
*after-each-tool* hook, passing the event on stdin. In Claude Code, add a `PostToolUse`
entry to `~/.claude/settings.json` — hook arrays are concatenated, so it coexists with
any other hooks you have:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "hooks": [
          { "type": "command", "command": "galdr hook" }
        ]
      }
    ]
  }
}
```

The sensor reads the harness fields from stdin (`tool_name`, `tool_input`,
`tool_response`, `cwd`, `session_id`, `transcript_path`) and is a no-op when no
recording is active.

If you prefer a resilient command that finds `galdr` on `PATH` and falls back to the
cargo bin, that form works too and is recognized by `galdr doctor` / `setup claude
--check`:

```sh
if command -v galdr >/dev/null 2>&1; then galdr hook; \
elif [ -x "$HOME/.cargo/bin/galdr" ]; then "$HOME/.cargo/bin/galdr" hook; fi
```

## Multi-harness: one skill, every harness

galdr distills a skill once, into the open-standard skills root (`~/.agents/skills`),
then makes it discoverable in **every harness installed on the machine**. Each harness
loads skills from its own directory, so galdr links the canonical skill into each one:

| Harness | Skills directory | Sensor wiring |
|---|---|---|
| Claude Code | `~/.claude/skills` | `~/.claude/settings.json` PostToolUse hook |
| Codex | `~/.codex/skills` | `~/.codex/hooks.json` (same hook shape) |
| Cursor | `~/.cursor/skills-cursor` | — |

The link is a symlink back to the canonical copy (the same mechanism a hand-linked
skill already uses), created on install, never clobbering a real file of the same
name. `galdr harnesses` shows what's installed; `galdr link` (re)links every skill;
`galdr doctor` flags any galdr skill a harness can't see. The result: record a task in
one harness, get a reusable skill in all of them.

## On-disk layout

```
~/.galdr/
├── active                      active recording (JSON); absent = not recording
├── config.json                 optional config (distill engine, endpoint, model)
├── galdrd.sock                 daemon control socket (NDJSON over a Unix socket)
├── galdrd.pid                  daemon pidfile
├── catalog.sqlite              queryable index, rebuilt from spans/ + recordings/ + skills
├── spans/<rec_id>.jsonl        append-only span, one JSON line per tool call
├── outcomes/skill_usage.jsonl  append-only skill usage observations
├── outcomes/skill_outcomes.jsonl append-only skill labels and reviews
└── recordings/<rec_id>.json    metadata for each closed recording
```

The span is the raw source of truth: append-only, immutable, inspectable. The SQLite
catalog is an **index, never the truth** — it stores one-line step summaries (no raw
blobs), readiness/evaluation rows, and outcome indexes. `galdr reindex` rebuilds it from
spans, recordings, skills, and outcome logs at any time.

The root is `~/.galdr` by default. Set `GALDR_ROOT` to relocate it (and
`GALDR_SKILLS_ROOT` to relocate the skills directory) — useful for hermetic tests,
throwaway profiles, CI, or to keep the daemon's Unix socket path under the platform's
length limit.

`config.json` may also include optional capture policy for future recordings:

```json
{
  "capture": {
    "deny_tools": ["SecretTool"],
    "deny_cwd_prefixes": ["/private/project"],
    "max_response_chars": 4000
  }
}
```

The policy only applies to new events as the hook records them. It never edits spans
that already exist.

## Extension points

The core exposes two generic seams (`src/ext.rs`) and nothing more:

- **`PermissionGate`** — decides whether an event may be recorded. Allows everything by
  default.
- **`ProvenanceSink`** — observes recorded events. Does nothing by default.

Any concrete integration (permission policy, traceability) plugs in by implementing
these traits from outside the repository.

## Security & privacy

The span captures each tool's `tool_input` and `tool_response`, which **may contain
sensitive data** (file contents, commands, outputs). Keep that in mind before recording
and before sharing a recording or a distilled skill. See [SECURITY.md](SECURITY.md).

Design guarantees:

- The raw lives **only** in `~/.galdr`, on your machine. galdr makes **no external
  network egress**. The one optional exception — autonomous distillation (`--auto`,
  feature `mlx`) — talks **only to loopback**, enforced in code by
  `engine::validate_loopback`; a non-loopback endpoint is a hard error.
- The sensor never propagates errors to the agent session, and never depends on the
  daemon: it appends to the span first, then hints the daemon best-effort.
- The autonomous distiller treats the recorded span as untrusted data (delimiter, low
  temperature, output validation, human review). Default distillation is still done by
  your own agent, locally.

## Roadmap

Phase 1 (shipped, built on the Phase 0 loop):

- ✅ **Supervisor daemon** + Unix socket + **SQLite** as a rebuildable catalog.
- ✅ **TUI** to browse recordings, inspect spans, and audit skill provenance.
- ✅ **Diff-based parametrization** of two recordings (constant = steps, variable = inputs).
- ✅ **Autonomous distillation** against a local, loopback-only MLX server.
- ✅ **Skill catalog readiness signals** with lifecycle status, provenance, score, delta,
  and an evaluator table for future human/LLM/model reviews.
- ✅ **Operational diagnostics** (`doctor`, `rec status`, daemon status/stop, Claude setup
  check/print).
- ✅ **Safe export path** that omits raw payloads by default and can emit redacted raw
  copies without touching the original span.

Phase 2 (shipped):

- ✅ **Finished in one** — `galdr distill` renders a complete, valid skill from the
  span (open-standard `When to use` / `Inputs` / `Steps` / `Verification` anatomy) and
  installs it, no agent pass required. `--draft` and `--auto` remain for a higher ceiling.
- ✅ **Multi-harness discoverability** — a distilled skill is linked into every installed
  harness's skills directory (Claude Code, Codex, Cursor); `galdr link` / `doctor` manage it.
- ✅ **Multi-harness sensor** — `galdr setup codex` wires the same hook into Codex's
  `hooks.json`; `galdr harnesses` shows which harnesses are wired.
- ✅ **Session-scoped recording** — a recording binds to the session that started it, so a
  concurrent session in another project can't leak its tool calls into the span.
- ✅ **AI-first CLI** — `--json` on every read command.

Next:

- **Capture of human GUI gestures** — the deliberate scope gap above. The agent's *own*
  browser automation is already captured (it arrives as tool calls). What's missing is
  recording a *human* driving a browser by hand, which needs a separate pixel/DOM capture
  layer on top of the span model — the one axis where Codex's pixel recorder does something
  galdr does not, and a job for the extension seam rather than the core.
- Verify the Codex sensor end to end with a live Codex recording (the wiring is in place;
  the stdin payload is not yet confirmed field-for-field).
- A multi-agent broker over the same span model.
- Real gates and real provenance plugged into the extension layer (`PermissionGate`,
  `ProvenanceSink`) — where harness-specific policy or memory integrations live, kept out
  of the local-first core.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits follow
[Conventional Commits](https://www.conventionalcommits.org/); the project uses
[Semantic Versioning](https://semver.org/).

## License

Apache-2.0. See [LICENSE](LICENSE).
