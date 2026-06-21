# Security Policy

## Data galdr handles

galdr records each tool call's `tool_input` and `tool_response`. These **can contain
sensitive data**: file contents, shell commands, command output, paths. Be aware of
this before you record a session and before you share a recording or a distilled skill.

Design guarantees:

- The raw span lives **only** under `~/.galdr` on the local machine.
- galdr makes **no external network egress**. Nothing is uploaded anywhere. The one
  optional exception is the autonomous distiller (`distill --auto`, built with the
  `mlx` feature), which talks **only to a loopback address** (`127.0.0.1`, `::1`,
  `localhost`). This is enforced in code by `engine::validate_loopback`: a
  non-loopback endpoint is a hard error, and the HTTP engine re-checks before every
  request. There is no configuration that points the distiller off the machine.
- The autonomous distiller treats the recorded span as **untrusted data**: it is
  wrapped in an explicit delimiter that tells the model never to follow instructions
  found inside, generation runs at a low temperature, the output is validated, and a
  human is expected to review the skill before use. Prefer reviewing any
  machine-generated skill before relying on it.
- The sensor (`galdr hook`) never propagates an error to the agent session.
- Exports omit raw tool payloads by default. `galdr export --include-raw` writes the
  original payloads and should be treated as sensitive; `galdr export --redact` writes
  a best-effort redacted copy without modifying the original span.
- Optional capture policy in `~/.galdr/config.json` can deny future events by tool name
  or CWD prefix, and can cap future recorded responses. It does not rewrite existing
  spans.

galdr does not redact secrets from a span. If a recorded session touched a credential,
treat the span as sensitive: it is a plain-text record on disk.

## Reporting a vulnerability

Please report security issues privately to **petruarakiss@gmail.com** with the subject
prefix `[galdr-security]`. Do not open a public issue for a vulnerability.

Include what you observed, how to reproduce it, and the impact. You can expect an initial
response within a few days.

## Scope

In scope: anything that lets the sensor break or alter an agent session, corrupt a span,
escape the local-only boundary (e.g. unexpected network egress, or the distiller reaching
a non-loopback host), or write outside the documented `~/.galdr` and skills directories.

Out of scope (for now): a hostile local user with the same OS account (they already have
filesystem access), and the sensitivity of data the operator chooses to record.

`galdr outcome` writes operator notes, usage outcomes, and review labels under
`~/.galdr/outcomes/`. Treat those files like spans: local-first and append-only, but
potentially sensitive if the notes include project names, incidents, or private context.
