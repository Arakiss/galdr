---
name: galdr-demo
description: "Summarize a git repository's recent changes and write them to a Markdown file. Use it when asked for a «change summary», «quick changelog», «what changed in the repo», or to dump git state to a file. Parameters: the repository and the output path."
---

# galdr-demo — git change summary to a file

Collects a git repository's status and latest commits and dumps them as a readable
Markdown summary. Distilled with galdr from a real recording.

## Goal

Given a git repository, produce a short Markdown file describing what changed: the
uncommitted working tree (status) and the latest commits (log). It serves as a quick
change note without opening the repo.

## Parameters

- `REPO` — path to the git repository. Defaults to the current working directory.
- `OUT` — path of the output Markdown file.
- `N` — number of commits to include (5 in the recording).

## Procedure

1. **Working tree status**: `git -C <REPO> status --short`. If the output is empty,
   there are no uncommitted changes; note that in the summary.
2. **Latest commits**: `git -C <REPO> log --oneline -<N>`.
3. **Write the summary** to `<OUT>` in this shape:

   ```markdown
   # Recent changes

   ## Uncommitted
   <status output, or "(clean tree)">

   ## Latest N commits
   <log output>
   ```

4. **Verify**: read `<OUT>` and check it has content (it did not come out empty).

## Success criteria

- Both git commands exit with code 0. If `<REPO>` is not a git repo, both fail: abort
  with a clear message.
- `<OUT>` exists and contains the two sections with real repo data.

## Robustness

- Precondition: `<REPO>` must be a git repository. Check it with
  `git -C <REPO> rev-parse` before steps 1-2.
- If `log` fails because the repo has no commits yet, write "(no commits yet)" in that
  section instead of aborting.
- Do not include absolute paths with sensitive data if you are going to share the
  summary.
