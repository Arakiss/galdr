# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
While the version is below `1.0.0`, breaking changes may land in minor releases.

## [Unreleased]

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

[Unreleased]: https://github.com/Arakiss/galdr/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Arakiss/galdr/releases/tag/v0.1.0
