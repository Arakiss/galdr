<p align="center">
  <img src="assets/banner.svg" alt="galdr — Record & Replay for agent skills" width="100%">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0">
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange.svg" alt="Rust 1.85+">
  <img src="https://img.shields.io/badge/status-phase%200%20·%20tracer%20bullet-8B7BF0.svg" alt="Status: phase 0">
  <img src="https://img.shields.io/badge/network-none%20·%20local--first-4FD6C9.svg" alt="Local-first, zero network">
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
> **Status: Phase 0 — tracer bullet.** This validates that the full loop closes
> (record → distill → replay) in the simplest possible setup: one binary, JSONL spans,
> distillation assisted by the agent itself. The daemon, the TUI, the SQLite catalog,
> and diff-based parametrization come later, on top of this proven base.

## Why tool calls, not pixels

A GUI Record & Replay records what the screen looked like. It breaks when a button
moves. An agent already emits a clean, structured trace of *what it did*: each tool
call, its input, and its result. galdr records that substrate. The replay is not a
pixel re-enactment — it is a skill the agent reads and applies with judgment.

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
3. **Distillation** (`galdr distill <id>`) — normalizes the span and emits a `SKILL.md`
   draft with instructions for the agent to complete by reading the raw span. The
   agent writes the refined skill to a working file; galdr installs it (galdr is the
   only writer of the skills directory).

## Quickstart

```sh
cargo install --path .

galdr rec start demo      # start recording
#  ... do the task with your agent (a few tool calls) ...
galdr rec stop            # close the recording, prints the rec_id

galdr list                # list recordings
galdr distill <rec_id>    # generate the skill draft
#  ... the agent reads the span and writes the refined skill to a temp file ...
galdr distill <rec_id> --from <temp-file>   # install the final skill
```

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

## On-disk layout

```
~/.galdr/
├── active                      active recording (JSON); absent = not recording
├── spans/<rec_id>.jsonl        append-only span, one JSON line per tool call
└── recordings/<rec_id>.json    metadata for each closed recording
```

The span is the raw source of truth: append-only, immutable, inspectable. A queryable
catalog (SQLite) arrives in a later phase and only *indexes* this raw — it never
replaces it.

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

- The raw lives **only** in `~/.galdr`, on your machine. galdr opens no network
  connection.
- The sensor never propagates errors to the agent session.
- Nothing is sent to any service: the Phase 0 distillation is done by your own agent,
  locally.

## Roadmap

Phase 0 is the tracer bullet. Built on this proven loop, later phases add:

- **Supervisor daemon** + Unix socket + **SQLite** as a queryable catalog.
- **TUI** to view, edit, replay, and audit recordings.
- **Diff-based parametrization** of two recordings (constant = steps, variable = inputs).
- **Autonomous distillation** (no agent assistance in the loop).
- Real gates and real provenance plugged into the extension layer.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits follow
[Conventional Commits](https://www.conventionalcommits.org/); the project uses
[Semantic Versioning](https://semver.org/).

## License

Apache-2.0. See [LICENSE](LICENSE).
