<!--
Thanks for the PR. Keep this template — maintainers scan it during review.

Conventional Commit: the PR title must match `<type>(<scope>): <subject>` where
type ∈ {feat, fix, chore, docs, refactor, perf, test, build, ci, revert,
security} and scope (when present) is one of the scopes listed in
.commitlintrc.yaml. CI will block the merge if it does not.
-->

## Summary

<!-- One or two sentences on what this PR does and why. -->

## Motivation

<!-- What problem does this solve, or what decision does it encode?
     Link the issue or discussion if one exists. -->

## Invariant impact

<!-- galdr has two load-bearing invariants. Tick what applies. -->

- [ ] No impact (docs, CI-only, tests that do not change behavior).
- [ ] **Sensor contract.** This PR touches `hook.rs`/`main.rs` or the recording
  path. The sensor still always exits 0 and never depends on the daemon; I ran
  the integration tests (`cargo test --test cli`) and `sensor_never_breaks_the_session`
  is green.
- [ ] **Loopback-only.** This PR touches `engine.rs`/`config.rs` or the distiller.
  No code path can reach a non-loopback host; `validate_loopback` still gates it.
- [ ] **The span stays immutable.** Nothing edits or deletes a recorded event; the
  catalog remains a rebuildable index, never the source of truth.

## Surface changes

<!-- Leave blank if none apply. -->

- [ ] Changes a CLI flag or the daemon IPC protocol (release notes flagged).
- [ ] Changes the on-disk layout under `~/.galdr` or the skills directory.
- [ ] Bumps a substrate dep (`ratatui`, `rusqlite`, `tokio`, `reqwest`) — full
  feature matrix run.

## Testing

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features mlx -- -D warnings
cargo test
cargo test --features mlx
```

## Notes for reviewers

<!-- Pitfalls, follow-ups, known limitations, alternatives considered and rejected. -->
