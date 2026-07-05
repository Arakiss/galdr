# galdr Roadmap — Demonstration Parity and the On-Policy Ledger

**Status:** living document · **Owner:** operator (Petru) · **Executor:** Codex engineering briefs, one worktree/branch per phase
**Last updated:** 2026-07-05

## North star

galdr is the local, model-agnostic Record & Replay ledger for agent skills:
it captures how work actually happened (agent tool streams and human
demonstrations), distills it into plain-markdown skills any harness can load,
and keeps the evidence loop honest (outcomes, judgments, bench, regression).

OpenAI's Codex app now ships a native Record & Replay feature (demonstrate a
workflow on macOS, Codex drafts a skill, replay via Computer Use / browser /
plugins). Two facts shape this roadmap:

1. **Parity is the bar.** Codex R&R defines what operators will expect from
   demonstration-to-skill tooling: guided recording, window-content awareness,
   a four-part skill anatomy (when to use / inputs / steps / verification),
   and refine-after-draft.
2. **The EEA gap is the opening.** Codex R&R initial availability excludes
   the European Economic Area, the UK, and Switzerland. galdr runs local-first
   with no such restriction. For EEA operators, galdr is not a clone — it is
   the only lane.

galdr's standing differentiators, never to be traded away for parity:
local-first and offline, git-friendly markdown skills, multi-harness
(Claude Code and Codex both consume the output), model-agnostic ledger
(galdr never calls LLM APIs), explicit privacy gates.

## What exists today (v0.17.0)

| Capability | Where |
|---|---|
| Agent tool-stream capture | `hook` (PostToolUse sensor), `rec` (multi-session, parallel-safe) |
| Human demonstration capture | `observe mac` (listen-only event tap: clicks, scrolls, keystrokes), `observe browser` (local CDP sensor), `observe synthetic` |
| Distillation | `distill` (recording → skill), `diff` + `parametrize` (two recordings → parameterized skill) |
| Evidence loop | `outcome`, `bench` (replay hit-rate), `evaluations` |
| Opportunity mining | `suggest` (repeated task shapes not yet distilled) |
| Distribution | `link`, `harnesses`, `skills`, `rm` |
| Safety | `validate` (content gate), `export` (payload redaction by default) |
| Infrastructure | `daemon`, `tui`, `reindex`, `doctor` |

## In flight

- **`feat/on-policy-judgments`** (Codex brief, 2026-07-05): per-step judgments
  ingested via CLI (`ok`/`fork` + rationale), fork-point sections in distilled
  skills, per-step outcome densification, skill regression guard. Derived from
  Thinking Machines' *On-Policy Distillation*: distill from the measured
  failure points of the executor that will replay the skill, not from the
  author's happy trajectory.

## Phases

Each phase is a self-contained Codex engineering brief: fresh worktree, own
branch, tests + CHANGELOG, parked branch, operator merges. Every phase must be
useful even if the roadmap stops after it.

### Phase 1 — Demonstration context capture

Codex R&R "observes the actions and window content"; `observe mac` today
captures input events without content context.

- Enrich `observe mac` events with per-action context: frontmost application,
  window title, and accessibility element role/label where the a11y API
  permits.
- Optional screenshot-at-action-boundary, behind an explicit opt-in flag and
  the content gate (redaction before persistence; never default-on).
- Recording intent metadata: declare the goal and the inputs that will vary
  between uses *before* recording starts (Codex asks for this up front);
  persist it on the recording.

### Phase 2 — Skill anatomy parity in distill

- `distill` accepts observation traces (mac/browser), not only agent spans.
- Generated skills follow the four-part anatomy: **when to use** (trigger
  conditions), **inputs** (the declared varying values), **steps**,
  **verification** (how to know it worked).
- Single-demo parametrization: use the declared varying inputs from Phase 1
  to parameterize from one recording (`parametrize` today requires two).

### Phase 3 — Guided operator flow

- One-command demonstration flow wrapping the whole loop: intent prompt →
  consent → observe → stop affordance (hotkey or menu-bar via the daemon) →
  auto-distill draft → hand the draft to the active harness for a refine
  conversation.
- Wire `suggest` output into "record this as a skill" prompts.

### Phase 4 — Replay surface bindings

- Skill frontmatter carries execution-surface hints (`cli` / `browser` /
  `gui`) so the consuming agent picks the right tools.
- Browser replay delegates to Portico (boundary: Portico executes and owns
  browser session memory; galdr remains the ledger and the skill format).
- GUI replay is the consuming agent's Computer Use problem, on purpose —
  galdr never executes workflows itself.

### Phase 5 — On-policy everywhere

- Judgments over observation traces: the human demo as teacher trajectory,
  weak-executor attempts as student; fork points measured against the demo.
- Per-step fork-rate aggregation in `bench`.
- Regression guard v2 informed by real usage of v1.

## Non-goals

- galdr never calls LLM APIs (judgments and drafts are ingested, not generated).
- galdr never executes workflows (harnesses and Portico do).
- No cloud dependency, no telemetry.
- Skills remain plain markdown — no proprietary bundle format.
