<p align="center">
  <img src="assets/banner.svg" alt="galdr — Record & Replay for agent skills" width="100%">
</p>

<p align="center">
  <a href="https://github.com/Arakiss/galdr/actions/workflows/ci.yml"><img src="https://github.com/Arakiss/galdr/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0">
  <img src="https://img.shields.io/badge/rust-1.88%2B-orange.svg" alt="Rust 1.88+">
  <img src="https://img.shields.io/badge/status-phase%201%20·%20daemon%20%2B%20TUI%20%2B%20diff-8B7BF0.svg" alt="Status: phase 1">
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
> truth), a terminal UI, diff-based parametrization of two recordings, and optional
> autonomous distillation against a local, loopback-only MLX server. A plain
> `cargo build` needs none of that — autonomous distillation is feature-gated and falls
> back cleanly to the agent-assisted draft.

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

## More commands

```sh
galdr daemon --detach        # run the supervisor daemon (catalog indexer + socket)
galdr show <rec_id>          # inspect one recording with its steps
galdr skills                 # list installed skills and their provenance
galdr reindex                # rebuild the SQLite catalog from disk
galdr tui                    # browse recordings, inspect spans, audit skills

galdr diff <a> <b>           # diff two recordings: constants vs parameters
galdr parametrize <a> <b> --emit   # write a parametrized SKILL.md (suffix -param)

galdr distill <rec_id> --auto      # autonomous distillation (local MLX, see below)
```

The daemon is optional. `list`/`show`/`skills` answer daemon-first, then from the
read-only catalog, then from a fresh in-memory index built straight from disk — so the
CLI works whether or not the daemon is running.

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

## On-disk layout

```
~/.galdr/
├── active                      active recording (JSON); absent = not recording
├── config.json                 optional config (distill engine, endpoint, model)
├── galdrd.sock                 daemon control socket (NDJSON over a Unix socket)
├── galdrd.pid                  daemon pidfile
├── catalog.sqlite              queryable index, rebuilt from spans/ + recordings/
├── spans/<rec_id>.jsonl        append-only span, one JSON line per tool call
└── recordings/<rec_id>.json    metadata for each closed recording
```

The span is the raw source of truth: append-only, immutable, inspectable. The SQLite
catalog is an **index, never the truth** — it stores one-line step summaries (no raw
blobs), and `galdr reindex` rebuilds it from the spans and recordings at any time.

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

Next:

- Opt-in capture of human GUI gestures.
- A multi-agent broker (Codex / Cursor) over the same span model.
- Real gates and real provenance plugged into the extension layer.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits follow
[Conventional Commits](https://www.conventionalcommits.org/); the project uses
[Semantic Versioning](https://semver.org/).

## License

Apache-2.0. See [LICENSE](LICENSE).
