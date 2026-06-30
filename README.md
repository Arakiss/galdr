<p align="center">
  <img src="assets/banner.svg" alt="galdr — Record & Replay for agent skills" width="100%">
</p>

<p align="center">
  <a href="https://crates.io/crates/galdr"><img src="https://img.shields.io/crates/v/galdr.svg" alt="crates.io"></a>
  <a href="https://github.com/Arakiss/galdr/actions/workflows/ci.yml"><img src="https://github.com/Arakiss/galdr/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust 1.88+">
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

<p align="center">
  <img src="assets/demo.gif" alt="galdr: record a task, distill it into a skill, surface opportunities, measure replay reliability" width="100%">
</p>

> [!NOTE]
> Around the core loop (record → distill → replay) galdr adds: a SQLite catalog (a
> rebuildable *index*, never the truth) with a supervisor daemon, an install-time
> content gate (blocks secrets, personal paths, dangerous commands), a terminal UI,
> safe export, diff-based **parametrization** of two recordings, **`galdr suggest`**
> (repeated tasks worth turning into a skill), **`galdr bench`** (per-skill replay
> hit-rate from recorded outcomes), and optional autonomous distillation against a
> local, loopback-only MLX server. A plain `cargo build` needs none of the MLX path —
> it is feature-gated and falls back cleanly to the faithful mechanical render.

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
Record & Replay *for what your agent does* — including its web automation and the GUI
work it drives through Computer Use (those actions are tool calls too; galdr keeps the
action and drops the screenshot).

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
3. **Distillation** (`galdr distill [reference]`) — a replay of the tool calls is not yet
   a skill. galdr renders a faithful draft from the span (real steps, secrets redacted)
   and hands the agent an authoring brief: read the span, supply the judgment galdr can't
   — the problem it solves, the inputs that vary, each step's intent, the gotchas — and
   install the authored skill with `--from`. galdr owns the mechanism; the agent owns the
   intelligence. `--fast` installs the mechanical render as-is in one step (a floor, for a
   human or a headless run); `--auto` lets a local MLX model author it.

## Install

```sh
# from crates.io
cargo install galdr

# or from source
cargo install --git https://github.com/Arakiss/galdr
```

Prefer a prebuilt binary? Every [release](https://github.com/Arakiss/galdr/releases/latest)
ships signed, checksummed binaries (Sigstore + SHA-256) and an SBOM for macOS and Linux
(arm64 + x86_64). No network egress at runtime — galdr only ever talks to loopback.

## Quickstart

```sh
galdr setup skill         # teach your harness(es) how to drive galdr (one time)

galdr rec start demo      # ● recording "demo" — now do the task with your agent
#  ... a few tool calls ...
galdr rec stop            # ■ stopped "demo" — 6 steps

galdr distill             # render a faithful draft of the most recent recording + an authoring brief
#  ... read the span, write the real skill, then ...
galdr distill --from skill.md   # install the skill you authored
```

galdr captures *what* ran; you supply *why*. The default hands you a faithful draft and a
brief — install your authored version with `--from`, or `galdr distill --fast` to accept
the mechanical render as-is (a floor, not a finished skill).

**You never copy a 26-character id.** Every command that takes a recording resolves it
the way you think about it — the most recent by default, or by name, or by a short id
prefix:

```sh
galdr distill demo        # draft the recording named "demo"
galdr show                # inspect the most recent recording's steps
galdr suggest             # repeated tasks worth turning into a skill
galdr bench               # how reliably your distilled skills actually replay
```

Run `galdr` with no arguments for a one-screen overview: whether anything is recording,
how many recordings and skills you have, and the next command to type.

## More commands

```sh
galdr daemon --detach        # run the supervisor daemon (catalog indexer + socket)
galdr daemon status          # check whether the daemon is answering
galdr daemon stop            # ask the daemon to shut down gracefully
galdr show [reference]       # inspect one recording (name, id prefix, or omit for the latest)
galdr skills                 # list installed skills, galdr/external origin, provenance, and readiness
galdr harnesses              # detect agent harnesses on this system and whether galdr's sensor is wired
galdr harnesses --json       # the same, machine-readable
galdr link                   # make distilled skills discoverable by every installed harness
galdr link --skill <name>    # link just one skill
galdr evaluations            # list skill evaluator outputs from the catalog
galdr evaluations --skill <name>   # show one skill's evaluator history
galdr outcome usage --skill <name> --outcome success   # --rec defaults to the latest recording
galdr outcome label --skill <name> --label accepted --evaluator human
galdr outcome list --skill <name>  # inspect captured usage/outcome labels
galdr suggest                # skill opportunities: repeated tasks not yet distilled
galdr suggest --min-count 1  # also surface single, undistilled recordings
galdr bench                  # replay reliability: per-skill hit-rate from recorded outcomes
galdr reindex                # rebuild the SQLite catalog from disk
galdr doctor                 # diagnose config, catalog, daemon, skills, and hook wiring
galdr setup claude --check   # check Claude Code PostToolUse hook wiring
galdr setup claude --print   # print the safe settings.json snippet
galdr setup codex --check    # check Codex PostToolUse hook wiring (~/.codex/hooks.json)
galdr setup codex --print    # print the safe Codex hooks snippet
galdr tui                    # lazygit-style browser: recordings, spans, skills, harnesses

galdr diff <a> <b>           # diff two recordings: constants vs parameters
galdr parametrize <a> <b> --emit   # write a parametrized SKILL.md (suffix -param)
galdr export [reference] --out ./export          # export metadata + summaries, no raw payloads
galdr export [reference] --out ./export --redact   # export a redacted raw copy

galdr distill --auto         # autonomous distillation of the latest recording (local MLX, see below)
```

galdr is two surfaces over one catalog, and **both serve humans and agents**. The CLI
gives a person warm, colorized output and a no-id-needed flow (resolve a recording by
the latest, a name, or a short prefix; `NO_COLOR` and non-TTY output are honored); it
gives an agent `--json` on every read command — `list`, `show`, `skills`, `evaluations`,
`harnesses`, `outcome list` — emitting a single parseable document, so a tool consumes
galdr without scraping a table:

```sh
galdr list --json | jq '.[].rec_id'
galdr skills --json | jq '[.[] | select(.origin == "galdr")]'
```

### Terminal UI

`galdr tui` is the browsable face — a lazygit-style cockpit over the same catalog, no
flags to memorize. Three panels (Recordings · Skills · Harnesses), moved with `1`/`2`/`3`
or `tab`, and a live preview that follows the selection:

- **Recordings** — `enter` steps into the span; `d` distills a complete skill; `e`
  exports it without raw payloads; `o` reveals the span path.
- **Skills** — `enter` reads the `SKILL.md` (scrollable); `l` links it into every
  installed harness; `v` validates it against the content gate; `O` records a success
  outcome (the signal `galdr bench` reads).
- `/` filters recordings and skills, `?` lists the keybindings, `q` quits.

Every action the TUI takes is **local and additive** — it distills, links, validates,
exports, and records outcomes. It never deletes a span, rewrites a skill behind your
back, or mutates your harness settings.

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

`galdr distill --auto` (optionally with a recording reference) lets a local model write
the finished skill instead of the agent. It is **off by default** and **loopback-only** —
it never reaches off the machine.

1. Build with the feature: `cargo install --path . --features mlx`.
2. Run a local OpenAI-compatible server, e.g. `mlx_lm.server` from
   [mlx-lm](https://github.com/ml-explore/mlx-lm):
   `python3 -m mlx_lm.server --model mlx-community/Qwen3-4B-Instruct-2507-4bit`
   (the default endpoint `http://127.0.0.1:8080` and model are configurable in
   `~/.galdr/config.json`).
3. `galdr distill --auto` (or `--engine mlx-subprocess` to shell out to
   `python3 -m mlx_lm.generate`, which needs no `mlx` feature).

If the engine is missing or unreachable, `--auto` falls back to the deterministic
complete skill and still exits 0. Always review a machine-generated skill before relying
on it.

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
| Codex | `~/.codex/skills` | `~/.codex/hooks.json` PostToolUse hook (must be trusted in Codex) |
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
    "max_response_chars": 4000,
    "strip_screenshots": true,
    "keep_frames": false
  }
}
```

The policy only applies to new events as the hook records them. It never edits spans
that already exist.

`strip_screenshots` (on by default) drops base64 image blobs — a Computer Use
screenshot, an image content block — from the recorded event, keeping the *action*
(click, type, key) but not the pixels. The pixels are large and may show sensitive
on-screen content, and they are never the reusable signal; the action is. This is what
lets galdr record a GUI task the agent drives via Computer Use and distill it into a
clean, semantic skill — the agent's GUI work is just more tool calls.

`keep_frames` (**off** by default) is the deliberate exception: with it on, a stripped
screenshot is also written as an **ephemeral** PNG under `~/.galdr/frames/<rec_id>/`, so
the authoring pass can *see* the screen and write better semantic steps for a GUI skill
("click the New Note button, top-left") instead of bare coordinates. `galdr distill`
surfaces the frames in its authoring brief, and they are **purged when a final skill
installs**. The frames never enter the span, the skill, or an export — pixels are
scaffolding to *produce* the skill, not part of it. It is opt-in precisely because it
puts pixels on disk; `galdr doctor` flags any that linger.

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

- ✅ **Faithful render** — `galdr distill` renders a valid skill from the span in the
  open-standard `When to use` / `Inputs` / `Steps` / `Verification` anatomy. `--fast`
  installs it as-is; `--auto` lets a local MLX model author it.
- ✅ **Multi-harness discoverability** — a distilled skill is linked into every installed
  harness's skills directory (Claude Code, Codex, Cursor); `galdr link` / `doctor` manage it.
- ✅ **Multi-harness sensor** — Codex has a native hooks system modeled on Claude Code's:
  the same `PostToolUse` event and stdin payload, so `galdr hook` reads it unchanged.
  `galdr setup codex --print` emits the snippet to merge into `~/.codex/hooks.json` and
  the trust step Codex requires (it skips an untrusted hook); `galdr harnesses` shows
  which harnesses are wired.
- ✅ **Session-scoped recording** — a recording binds to the session that started it, so a
  concurrent session in another project can't leak its tool calls into the span.
- ✅ **AI-first CLI** — `--json` on every read command.

Phase 3 (shipped) — a surface humans and agents both want to use:

- ✅ **Author by default** — a replay of the tool calls is not a skill. `galdr distill`
  now renders a faithful draft and hands the agent an authoring brief (supply the why,
  the generalized inputs, the gotchas), installed with `--from`; an unauthored draft
  scores lower until it is raised. `--fast` keeps the mechanical one-shot for a floor.
- ✅ **No ids to copy** — every command resolves a recording by the most recent, a name,
  or a short id prefix. `galdr distill` distills the run you just made.
- ✅ **A friendly home screen** — `galdr` with no arguments shows where you are and the
  next command to type; output is colorized, TTY-aware, and honors `NO_COLOR`.
- ✅ **`galdr suggest`** — signs every recording by the shape of its steps, dedupes
  against installed skills, and ranks the repeated tasks worth distilling.
- ✅ **`galdr bench`** — aggregates recorded outcomes into a per-skill replay hit-rate
  and effort cost; the production signal a capability test cannot give.
- ✅ **A lazygit-style TUI** — three panels, a live preview, and in-place actions
  (distill, link, validate, export, record outcome), all local and additive.

Next:

- **Capture of human GUI gestures** — the deliberate scope gap above. The agent's *own*
  browser automation is already captured (it arrives as tool calls). What's missing is
  recording a *human* driving a browser by hand, which needs a separate pixel/DOM capture
  layer on top of the span model — the one axis where Codex's pixel recorder does something
  galdr does not, and a job for the extension seam rather than the core.
- Verify the Codex sensor end to end with a live Codex recording. The payload is now
  confirmed compatible (Codex's native `PostToolUse` carries the same `tool_name` /
  `tool_input` / `tool_response` fields), so what remains is a live capture once the hook
  is merged and trusted (`/hooks`).
- A multi-agent broker over the same span model.
- Real gates and real provenance plugged into the extension layer (`PermissionGate`,
  `ProvenanceSink`) — where harness-specific policy or memory integrations live, kept out
  of the local-first core.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits follow
[Conventional Commits](https://www.conventionalcommits.org/); the project uses
[Semantic Versioning](https://semver.org/).

## License

[MIT](LICENSE) © Petru Arakiss. Simple and permissive: use it, fork it, ship it.
