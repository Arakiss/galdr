# Contributing to galdr

Thanks for your interest. galdr is in Phase 0 (tracer bullet); the surface is small and
the bar is high on two things: the sensor must never break a session, and the raw span
must stay an immutable source of truth.

## Build and test

```sh
cargo build
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test
```

## Ground rules

- **The sensor never propagates failure.** `galdr hook` must always exit 0. Do not add
  a code path that can break the agent session. The `catch_unwind` guard in `main` and
  the `Result`-discarding contract in `hook.rs` are load-bearing — keep them.
- **The span is append-only and immutable.** Nothing edits or deletes a recorded event.
  Downstream stores (a future SQLite catalog) index the raw; they never replace it.
- **The public core stays generic.** Integrations plug in through the `PermissionGate`
  and `ProvenanceSink` traits, from outside this repository. Do not hardcode any
  specific harness, policy engine, or memory system into the core.
- **All code, comments, and docs are in English.**

## Commits

This project uses [Conventional Commits](https://www.conventionalcommits.org/):
`<type>(<scope>): <subject>`.

- **Types**: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`.
- **Scopes** (when useful): `sensor`, `record`, `span`, `distill`, `cli`, `paths`,
  `ext`, `docs`.
- Subject in the imperative mood, ≤ 72 characters, no trailing period.
- Breaking changes: `feat(scope)!:` with a `BREAKING CHANGE:` footer.
- Keep commits atomic — one concern per commit.

## Versioning

[Semantic Versioning](https://semver.org/). Below `1.0.0`, breaking API/CLI/schema
changes are released as minor bumps; compatible fixes as patch bumps. Update
`CHANGELOG.md` under `## [Unreleased]` with your change.
