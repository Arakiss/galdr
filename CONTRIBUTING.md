# Contributing to galdr

Thanks for considering a contribution. galdr's bar is high on two invariants:
the **sensor must never break an agent session**, and the **core makes no external
network egress**. Everything else follows from those.

## Ground rules

1. **The sensor never propagates failure.** `galdr hook` must always exit 0. The
   `catch_unwind` guard in `main` and the `Result`-discarding contract in `hook.rs`
   are load-bearing — keep them. The sensor also never *depends* on the daemon: it
   appends to the span first (the truth, unconditional), then hints the daemon
   best-effort. The integration test `sensor_never_breaks_the_session` guards this.
2. **No external network egress in the core.** The only optional traffic is the
   autonomous distiller (`distill --auto`, feature `mlx`), and it talks **only to
   loopback** — enforced by `engine::validate_loopback`, which the HTTP engine
   re-checks before every request. Do not add any other network path.
3. **The span is append-only and immutable.** Nothing edits or deletes a recorded
   event. The SQLite catalog is a rebuildable *index, never the source of truth*;
   `galdr reindex` recreates it from `spans/` + `recordings/`.
4. **galdr is the only writer of the skills directory.** New install paths go
   through the shared `install_skill`; the agent never writes `~/.agents/skills`
   by hand.
5. **The public core stays generic.** Integrations plug in through the
   `PermissionGate` and `ProvenanceSink` traits, from outside this repository. Do
   not hardcode any specific harness, policy engine, or memory system into the core.
6. **All code, comments, and docs are in English.**

## Commit convention

We use [Conventional Commits](https://www.conventionalcommits.org/). CI runs
`commitlint` on every PR; see `.commitlintrc.yaml` for the exact ruleset.

**Types** (required): `feat`, `fix`, `chore`, `docs`, `refactor`, `perf`, `test`,
`build`, `ci`, `revert`, `security`.

**Scope** (optional, but when present must be one of): `paths`, `span`, `record`,
`hook`, `sensor`, `ext`, `summary`, `catalog`, `ipc`, `daemon`, `tui`, `diff`,
`parametrize`, `distill`, `engine`, `config`, `cli`, `docs`, `ci`, `deps`,
`release`, `security`, `examples`.

Examples:

```
feat(daemon): reconcile dropped notifications in the poll-watcher
fix(catalog): keep the ended_at NULL filter out of the in-memory fallback
security(engine): re-check validate_loopback before every HTTP request
chore(deps): bump rusqlite to 0.40.2 — feature matrix green
```

**Breaking changes**: add `!` after the type (`feat(cli)!:`) and include a
`BREAKING CHANGE:` footer.

## Versioning

[Semantic Versioning](https://semver.org/). Below `1.0.0`, breaking changes to a
CLI flag, the daemon IPC protocol, or the on-disk layout are released as minor
bumps, with the migration path documented in the release notes; compatible fixes
are patch bumps.

## Releases

Automated via [release-please](https://github.com/googleapis/release-please). Do
not tag manually: the bot opens a release PR as Conventional Commits land, and
merging it creates the `v*` tag and triggers the signed binary build.

## Local dev

`just check` runs the same gate as CI (`just` from <https://just.systems>):

```sh
just check          # fmt + clippy (both features) + test (both features) + deny
```

Or run the steps directly:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features mlx -- -D warnings
cargo test
cargo test --features mlx
cargo deny check    # requires `cargo install cargo-deny`
```

The integration tests under `tests/cli.rs` drive the compiled binary in an
isolated temp `HOME`, covering the sensor contract, the daemon round-trip, the
catalog fallbacks, and diff/parametrize end to end.

## Style

- `rustfmt` defaults, no custom `rustfmt.toml` unless we outgrow them.
- `clippy::pedantic` not enforced globally; apply where it clarifies, skip where it
  produces noise.
- Doc comments on public items; module docs explain the *why*, not just the *what*.
