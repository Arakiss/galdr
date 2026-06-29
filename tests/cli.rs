//! End-to-end tests that drive the compiled `galdr` binary.
//!
//! Each test runs in its own temporary `HOME`, so `~/.galdr` and `~/.agents` are
//! isolated and the tests are hermetic and parallel-safe. The binary path comes
//! from `CARGO_BIN_EXE_galdr`, which cargo sets for integration tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_galdr")
}

/// An isolated `HOME` with a `galdr` command builder.
struct Sandbox {
    home: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn cmd(&self) -> Command {
        let mut command = Command::new(bin());
        command.env("HOME", self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    /// Feeds a PostToolUse event to the sensor on stdin.
    fn hook(&self, json: &str, fail: bool) -> Output {
        let mut command = self.cmd();
        command.arg("hook");
        if fail {
            command.env("GALDR_HOOK_FAIL", "1");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn span_lines(&self, rec_id: &str) -> usize {
        let path = self
            .home()
            .join(".galdr/spans")
            .join(format!("{rec_id}.jsonl"));
        std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }

    /// The rec_id of the in-progress recording (read from the `active` flag,
    /// since `recordings/` is only written on stop).
    fn active_rec_id(&self) -> String {
        let raw = std::fs::read_to_string(self.home().join(".galdr/active")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["rec_id"].as_str().unwrap().to_string()
    }

    fn recording_ids(&self) -> Vec<String> {
        let dir = self.home().join(".galdr/recordings");
        let mut ids: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.extension()?.to_str()? == "json" {
                            Some(path.file_stem()?.to_str()?.to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// Records a sequence of events under `name` and returns the rec_id.
    fn record(&self, name: &str, events: &[&str]) -> String {
        let before = self.recording_ids();
        assert!(self.run(&["rec", "start", name]).status.success());
        for event in events {
            assert!(self.hook(event, false).status.success());
        }
        assert!(self.run(&["rec", "stop"]).status.success());
        self.recording_ids()
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("a new recording id")
    }

    fn skill_md(&self, skill_name: &str) -> String {
        let path = self
            .home()
            .join(".agents/skills")
            .join(skill_name)
            .join("SKILL.md");
        std::fs::read_to_string(path).unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const BASH_STATUS: &str =
    r#"{"tool_name":"Bash","tool_input":{"command":"git status"},"tool_response":{}}"#;

#[test]
fn json_output_is_machine_readable() {
    // The CLI is the AI-first surface: every --json flag must emit a single,
    // parseable JSON document an agent can consume without scraping a table.
    let sb = Sandbox::new();
    let id = sb.record("json task", &[BASH_STATUS]);

    let refined = sb.home().join("r.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-json-task\ndescription: \"json\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    assert!(
        sb.cmd()
            .args(["distill", &id, "--from"])
            .arg(&refined)
            .output()
            .unwrap()
            .status
            .success()
    );

    let parse = |args: &[&str]| -> serde_json::Value {
        let out = sb.run(args);
        assert!(out.status.success(), "{args:?} failed");
        serde_json::from_str(&stdout(&out))
            .unwrap_or_else(|e| panic!("{args:?} did not emit valid JSON: {e}"))
    };

    // list → array with our recording
    let list = parse(&["list", "--json"]);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|r| r["rec_id"] == id.as_str())
    );

    // show → object with steps
    let show = parse(&["show", &id, "--json"]);
    assert_eq!(show["recording"]["name"], "json task");
    assert_eq!(show["steps"].as_array().unwrap().len(), 1);

    // skills → array carrying the origin classification
    let skills = parse(&["skills", "--json"]);
    let skill = skills
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["skill_name"] == "galdr-json-task")
        .expect("the distilled skill is listed");
    assert_eq!(skill["origin"], "galdr");

    // harnesses → array, always non-empty (the known set)
    let harnesses = parse(&["harnesses", "--json"]);
    assert!(!harnesses.as_array().unwrap().is_empty());
    assert!(
        harnesses
            .as_array()
            .unwrap()
            .iter()
            .any(|h| h["key"] == "claude")
    );

    // outcome list → object with usage/labels keys
    assert!(
        sb.run(&[
            "outcome",
            "usage",
            "--skill",
            "galdr-json-task",
            "--rec",
            &id,
            "--outcome",
            "success",
        ])
        .status
        .success()
    );
    let outcomes = parse(&["outcome", "list", "--json"]);
    assert!(outcomes["usage"].is_array());
    assert!(outcomes["labels"].is_array());
}

#[test]
fn default_distill_produces_a_complete_usable_skill() {
    // The "finished in one" bar: `galdr distill <id>` with no flags must install a
    // complete, valid skill in the open-standard anatomy — no agent pass, no draft.
    let sb = Sandbox::new();
    let id = sb.record(
        "deploy preview",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Write","tool_input":{"file_path":"/repo/out.txt"},"tool_response":{}}"#,
        ],
    );
    assert!(sb.run(&["distill", &id]).status.success());

    let skill = sb.skill_md("galdr-deploy-preview");
    for section in ["## When to use", "## Inputs", "## Steps", "## Verification"] {
        assert!(skill.contains(section), "missing {section}:\n{skill}");
    }
    assert!(!skill.contains("[galdr DRAFT]"));
    assert!(!skill.contains("TODO(agent)"));
    // It scores as a complete, ready skill — not a draft.
    let listing = stdout(&sb.run(&["skills"]));
    assert!(listing.contains("final"));
    assert!(listing.contains("ready"));
}

#[test]
fn distill_name_chooses_the_skill_name() {
    // galdr supplies the mechanism; the caller brings the naming intelligence. `--name`
    // installs under a chosen, memorable name instead of the mechanical galdr-<slug>.
    let sb = Sandbox::new();
    let id = sb.record("whatever the recording was called", &[BASH_STATUS]);
    assert!(
        sb.run(&["distill", &id, "--name", "rust-greenlight"])
            .status
            .success()
    );
    let md = sb.skill_md("rust-greenlight");
    assert!(md.contains("name: rust-greenlight"), "{md}");
    assert!(
        !sb.home()
            .join(".agents/skills/galdr-whatever-the-recording-was-called")
            .exists(),
        "the mechanical name must not also be created"
    );
    // It still validates and is classified as a galdr skill (origin is content-based,
    // not the name prefix), so dropping the prefix is safe.
    assert!(sb.run(&["validate", "rust-greenlight"]).status.success());
}

#[test]
fn validate_passes_clean_skills_and_refuses_bad_content() {
    // The gate is reachable from the CLI: a clean distilled skill validates, a file
    // carrying a personal path is refused, and --all --json is machine-readable.
    let sb = Sandbox::new();
    let id = sb.record("validate demo", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &id]).status.success());

    let ok = sb.run(&["validate", "galdr-validate-demo"]);
    assert!(
        ok.status.success(),
        "a clean distilled skill must pass: {}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let all = sb.run(&["validate", "--all", "--json"]);
    assert!(all.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&all)).unwrap();
    assert!(parsed.is_array(), "--json emits an array: {}", stdout(&all));

    // A file with a personal path is a hard security failure (exit non-zero).
    let bad = sb.home().join("bad.md");
    std::fs::write(
        &bad,
        "---\nname: galdr-bad\ndescription: \"x\"\n---\n\n## When to use\n\nx\n\n## Steps\n\n1. **Read** — /Users/alice/secret.txt\n\n## Verification\n\ny\n",
    )
    .unwrap();
    let refused = sb
        .cmd()
        .args(["validate", "--file"])
        .arg(&bad)
        .output()
        .unwrap();
    assert!(
        !refused.status.success(),
        "a personal path must fail the gate"
    );
}

#[test]
fn setup_codex_check_and_print_work() {
    let sb = Sandbox::new();
    let missing = stdout(&sb.run(&["setup", "codex", "--check"]));
    assert!(missing.contains("not found"));

    let snippet = stdout(&sb.run(&["setup", "codex", "--print"]));
    assert!(snippet.contains("PostToolUse"));
    assert!(snippet.contains("galdr hook"));

    let hooks = sb.home().join(".codex/hooks.json");
    std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    std::fs::write(&hooks, snippet).unwrap();
    let configured = stdout(&sb.run(&["setup", "codex", "--check"]));
    assert!(configured.contains("is configured"));
}

#[test]
fn distilled_skill_is_linked_into_installed_harnesses() {
    // The make-or-break for "R/R for Claude Code": a distilled skill must become
    // discoverable in the harness it was recorded in, not dead-end in the open
    // standard root the harness never reads.
    let sb = Sandbox::new();
    // Stand up a Claude Code skills dir so the harness is "installed" and known.
    std::fs::create_dir_all(sb.home().join(".claude/skills")).unwrap();

    let id = sb.record("link task", &[BASH_STATUS]);
    let refined = sb.home().join("r.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-link-task\ndescription: \"link\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    assert!(
        sb.cmd()
            .args(["distill", &id, "--from"])
            .arg(&refined)
            .output()
            .unwrap()
            .status
            .success()
    );

    // The skill is now reachable through the Claude Code skills directory.
    let linked = sb.home().join(".claude/skills/galdr-link-task/SKILL.md");
    assert!(
        linked.exists(),
        "the distilled skill must be discoverable in ~/.claude/skills"
    );
    // And it resolves back to the canonical open-standard copy.
    let canonical = sb.home().join(".agents/skills/galdr-link-task/SKILL.md");
    assert!(canonical.exists());
}

#[test]
fn link_never_clobbers_a_real_skill_already_in_the_harness() {
    let sb = Sandbox::new();
    // A user's own, hand-authored skill of the same name already lives in Claude Code.
    let existing = sb.home().join(".claude/skills/galdr-keepme");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("SKILL.md"), "real user content").unwrap();

    let id = sb.record("keepme", &[BASH_STATUS]);
    let refined = sb.home().join("r.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-keepme\ndescription: \"x\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    assert!(
        sb.cmd()
            .args(["distill", &id, "--from"])
            .arg(&refined)
            .output()
            .unwrap()
            .status
            .success()
    );

    // The user's real file is untouched (not replaced by a symlink).
    let content =
        std::fs::read_to_string(sb.home().join(".claude/skills/galdr-keepme/SKILL.md")).unwrap();
    assert_eq!(content, "real user content");
    assert!(
        !sb.home()
            .join(".claude/skills/galdr-keepme")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );

    // `galdr link --json` reports the conflict rather than silently failing.
    let out = sb.run(&["link", "--skill", "galdr-keepme", "--json"]);
    let results: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(
        results
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["harness"] == "Claude Code" && r["status"] == "conflict")
    );
}

#[test]
fn link_rejects_path_traversal_in_skill_name() {
    // `galdr link --skill ../x` must not escape the skills root to create a symlink
    // at an arbitrary sibling path.
    let sb = Sandbox::new();
    let out = sb.run(&["link", "--skill", "../evil"]);
    assert!(
        !out.status.success(),
        "path-traversal skill name must be rejected"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("path separator") || err.contains("invalid skill name"),
        "{err}"
    );
    // Nothing got created outside the skills dir.
    assert!(!sb.home().join(".claude/evil").exists());
}

#[test]
fn export_redact_scrubs_secrets_from_every_file_not_just_raw() {
    // The worst redaction bug: --redact scrubbed raw.redacted.jsonl but left the
    // secret in steps.md (the Bash command summary). It must scrub all files.
    let sb = Sandbox::new();
    let id = sb.record(
        "leaky",
        &[r#"{"tool_name":"Bash","tool_input":{"command":"curl -H 'Authorization: Bearer ghp_SECRETtoken123' https://api"},"tool_response":{}}"#],
    );
    let out = sb.home().join("exp");
    assert!(
        sb.cmd()
            .args(["export", &id, "--out"])
            .arg(&out)
            .arg("--redact")
            .output()
            .unwrap()
            .status
            .success()
    );
    for file in ["steps.md", "raw.redacted.jsonl"] {
        let content = std::fs::read_to_string(out.join(file)).unwrap();
        assert!(
            !content.contains("ghp_SECRETtoken123"),
            "{file} still leaks the secret:\n{content}"
        );
    }
    assert!(
        std::fs::read_to_string(out.join("steps.md"))
            .unwrap()
            .contains("[REDACTED]")
    );
}

#[test]
fn galdr_root_is_locked_to_the_owner() {
    // Spans hold raw tool data; another local user must not be able to read them.
    let sb = Sandbox::new();
    sb.record("private", &[BASH_STATUS]);
    let meta = std::fs::metadata(sb.home().join(".galdr")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        meta.permissions().mode() & 0o077,
        0,
        "~/.galdr must be 0700 (no group/other access)"
    );
}

#[test]
fn hook_survives_an_oversized_payload() {
    // A hostile/huge stdin must not crash the sensor; it caps the read and drops the
    // (truncated, unparseable) event, still exiting 0.
    let sb = Sandbox::new();
    assert!(sb.run(&["rec", "start", "big"]).status.success());
    let id = sb.active_rec_id();
    let huge = format!(
        r#"{{"tool_name":"Bash","tool_input":{{"command":"{}"}},"tool_response":{{}}}}"#,
        "A".repeat(2_000_000)
    );
    let out = sb.hook(&huge, false);
    assert!(out.status.success(), "the sensor must always exit 0");
    // A 2 MB payload is under the cap, so it records; the point is it does not crash.
    assert!(sb.span_lines(&id) <= 1);
}

#[test]
fn computer_use_screenshots_are_dropped_but_actions_recorded() {
    // galdr captures an agent's Computer Use as tool calls, but the screenshot (a big
    // base64 image, possibly sensitive) is dropped from the span — only the action stays.
    let sb = Sandbox::new();
    assert!(sb.run(&["rec", "start", "gui task"]).status.success());
    let id = sb.active_rec_id();
    let blob = "iVBORw0KGgoAAAANSUhEUg".repeat(80);
    let shot = format!(
        r#"{{"tool_name":"mcp__computer-use__computer","tool_input":{{"action":"screenshot"}},"tool_response":{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{blob}"}}}},"session_id":"s1"}}"#
    );
    assert!(sb.hook(&shot, false).status.success());
    assert!(
        sb.hook(
            r#"{"tool_name":"mcp__computer-use__computer","tool_input":{"action":"type","text":"42.50"},"tool_response":{},"session_id":"s1"}"#,
            false,
        )
        .status
        .success()
    );
    assert!(sb.run(&["rec", "stop"]).status.success());

    let span = std::fs::read_to_string(sb.home().join(".galdr/spans").join(format!("{id}.jsonl")))
        .unwrap();
    assert!(
        !span.contains("iVBORw0KGgo"),
        "the screenshot base64 must be dropped"
    );
    assert!(span.contains("stripped screenshot"));

    // The actions read cleanly in `show`.
    let show = stdout(&sb.run(&["show", &id]));
    assert!(show.contains("screenshot"));
    assert!(show.contains("type \"42.50\""), "got: {show}");
}

#[test]
fn a_typed_secret_is_redacted_from_the_distilled_skill() {
    // A Computer Use `type` of a token must not be promoted into the installed,
    // shareable SKILL.md (Inputs or Steps).
    let sb = Sandbox::new();
    let id = sb.record(
        "login flow",
        &[
            r#"{"tool_name":"mcp__computer-use__computer","tool_input":{"action":"type","text":"ghp_SUPERSECRETtoken123"},"tool_response":{}}"#,
        ],
    );
    assert!(sb.run(&["distill", &id]).status.success());
    let skill = sb.skill_md("galdr-login-flow");
    assert!(
        !skill.contains("ghp_SUPERSECRETtoken123"),
        "secret leaked into skill:\n{skill}"
    );
    assert!(skill.contains("[REDACTED]"));
}

#[test]
fn sensor_never_breaks_the_session() {
    let sb = Sandbox::new();

    // No active recording: a no-op, still exit 0.
    assert!(sb.hook(BASH_STATUS, false).status.success());

    assert!(sb.run(&["rec", "start", "demo"]).status.success());
    let id = sb.active_rec_id();

    // Active recording: appends and exits 0.
    assert!(sb.hook(BASH_STATUS, false).status.success());
    assert_eq!(sb.span_lines(&id), 1);

    // Forced internal failure: still exit 0, and nothing appended.
    let failed = sb.hook(BASH_STATUS, true);
    assert!(failed.status.success(), "the sensor must always exit 0");
    assert_eq!(sb.span_lines(&id), 1, "a failed hook must not append");
}

#[test]
fn recording_scopes_to_the_session_that_started_it() {
    // A single global `active` flag means every concurrent agent session's hook
    // sees this recording. The sensor must bind to the starting session and refuse
    // events from another session, so a parallel session in another project cannot
    // leak its tool calls into this span.
    let sb = Sandbox::new();
    assert!(sb.run(&["rec", "start", "scoped"]).status.success());
    let id = sb.active_rec_id();

    // First event carrying a session id binds the recording (no cwd → binds).
    assert!(
        sb.hook(
            r#"{"tool_name":"Bash","tool_input":{"command":"mine-1"},"tool_response":{},"session_id":"mine"}"#,
            false,
        )
        .status
        .success()
    );
    // A different session's event, in another directory, must be dropped.
    assert!(
        sb.hook(
            r#"{"tool_name":"Bash","tool_input":{"command":"leak"},"tool_response":{},"session_id":"other","cwd":"/elsewhere"}"#,
            false,
        )
        .status
        .success()
    );
    // The bound session keeps recording.
    assert!(
        sb.hook(
            r#"{"tool_name":"Read","tool_input":{"file_path":"/x"},"tool_response":{},"session_id":"mine"}"#,
            false,
        )
        .status
        .success()
    );

    assert_eq!(
        sb.span_lines(&id),
        2,
        "only the bound session's events record"
    );
    let span = std::fs::read_to_string(sb.home().join(".galdr/spans").join(format!("{id}.jsonl")))
        .unwrap();
    assert!(
        !span.contains("leak"),
        "the foreign session's command must not leak in: {span}"
    );
    assert!(!span.contains("\"other\""));
}

#[test]
fn record_list_show_work_without_a_daemon() {
    let sb = Sandbox::new();
    let id = sb.record(
        "demo task",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Write","tool_input":{"file_path":"/tmp/out.md"},"tool_response":{}}"#,
        ],
    );

    let list = sb.run(&["list"]);
    assert!(list.status.success());
    let listing = stdout(&list);
    assert!(
        listing.contains("demo task"),
        "list shows the name: {listing}"
    );
    assert!(listing.contains("2 steps"), "list shows the step count");

    let show = sb.run(&["show", &id]);
    assert!(show.status.success());
    let detail = stdout(&show);
    assert!(detail.contains("Bash"));
    assert!(detail.contains("Write"));
    assert!(detail.contains("git status"));
}

#[test]
fn distill_from_installs_and_skills_lists_provenance() {
    let sb = Sandbox::new();
    let id = sb.record("demo", &[BASH_STATUS]);

    // Install a finished skill through the sanctioned --from path. A distilled
    // skill keeps its provenance line, so the rebuilt catalog can link it back.
    let refined = sb.home().join("refined.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-demo\ndescription: \"does a thing\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let install = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .output()
        .unwrap();
    assert!(install.status.success());

    let skills = sb.run(&["skills"]);
    assert!(skills.status.success());
    let listing = stdout(&skills);
    assert!(
        listing.contains("galdr-demo"),
        "skills lists the skill: {listing}"
    );
    // Provenance links to the recording, not flagged orphan.
    assert!(listing.contains(&id));
    assert!(!listing.contains("orphan"));
}

#[test]
fn reindex_rebuilds_the_catalog_from_disk() {
    let sb = Sandbox::new();
    sb.record("demo", &[BASH_STATUS]);

    let reindex = sb.run(&["reindex"]);
    assert!(reindex.status.success());
    assert!(stdout(&reindex).contains("catalog rebuilt"));
    // The catalog file now exists and was rebuilt from disk.
    assert!(sb.home().join(".galdr/catalog.sqlite").exists());

    let list = sb.run(&["list"]);
    assert!(list.status.success());
    assert!(stdout(&list).contains("demo"));
}

#[test]
fn recording_writes_keep_an_existing_catalog_current_without_a_daemon() {
    let sb = Sandbox::new();
    sb.record("first", &[BASH_STATUS]);
    assert!(sb.run(&["reindex"]).status.success());

    let second = sb.record(
        "second",
        &[r#"{"tool_name":"Read","tool_input":{"file_path":"/tmp/input.md"},"tool_response":{}}"#],
    );

    let list = sb.run(&["list"]);
    assert!(list.status.success());
    let listing = stdout(&list);
    assert!(
        listing.contains("second"),
        "list should not read a stale catalog: {listing}"
    );

    let show = sb.run(&["show", &second]);
    assert!(show.status.success());
    let detail = stdout(&show);
    assert!(
        detail.contains("/tmp/input.md"),
        "show should include the newly indexed step: {detail}"
    );
}

#[test]
fn skill_writes_keep_an_existing_catalog_current_without_a_daemon() {
    let sb = Sandbox::new();
    let id = sb.record("stale catalog", &[BASH_STATUS]);
    assert!(sb.run(&["reindex"]).status.success());

    let refined = sb.home().join("refined.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-stale-catalog\ndescription: \"stale catalog check\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let install = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .output()
        .unwrap();
    assert!(install.status.success());

    let skills = sb.run(&["skills"]);
    assert!(skills.status.success());
    let listing = stdout(&skills);
    assert!(
        listing.contains("galdr-stale-catalog"),
        "skills should not read a stale catalog: {listing}"
    );
    assert!(listing.contains(&id));
}

#[test]
fn draft_distill_keeps_an_existing_catalog_current_without_a_daemon() {
    let sb = Sandbox::new();
    let id = sb.record("draft catalog", &[BASH_STATUS]);
    assert!(sb.run(&["reindex"]).status.success());

    let draft = sb.run(&["distill", &id]);
    assert!(draft.status.success());

    let skills = sb.run(&["skills"]);
    assert!(skills.status.success());
    let listing = stdout(&skills);
    assert!(
        listing.contains("galdr-draft-catalog"),
        "draft distillation should update an existing catalog: {listing}"
    );
    assert!(listing.contains(&id));
}

#[test]
fn parametrize_emit_keeps_an_existing_catalog_current_without_a_daemon() {
    let sb = Sandbox::new();
    let write = |path: &str| {
        format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{path}"}},"tool_response":{{}}}}"#
        )
    };
    let a = sb.record("ship", &[BASH_STATUS, &write("/repo-a/out.md")]);
    let b = sb.record("ship", &[BASH_STATUS, &write("/repo-b/out.md")]);
    assert!(sb.run(&["reindex"]).status.success());

    let emit = sb.run(&["parametrize", &a, &b, "--emit"]);
    assert!(emit.status.success());

    let skills = sb.run(&["skills"]);
    assert!(skills.status.success());
    let listing = stdout(&skills);
    assert!(
        listing.contains("galdr-ship-param"),
        "parametrize should update an existing catalog: {listing}"
    );
    assert!(listing.contains(&a));
}

#[test]
fn parametrize_emits_a_templated_skill() {
    let sb = Sandbox::new();
    let write = |path: &str| {
        format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{path}"}},"tool_response":{{}}}}"#
        )
    };
    let a = sb.record("ship", &[BASH_STATUS, &write("/repo-a/out.md")]);
    let b = sb.record("ship", &[BASH_STATUS, &write("/repo-b/out.md")]);

    let emit = sb.run(&["parametrize", &a, &b, "--emit"]);
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );

    let skill = sb.skill_md("galdr-ship-param");
    assert!(skill.contains("## Parameters"));
    assert!(skill.contains("## Procedure (parametrized)"));
    assert!(skill.contains("{{OUT}}"), "the output path is templated");
    assert!(
        !skill.contains("LOW-CONFIDENCE"),
        "a clean alignment is high confidence"
    );
}

#[test]
fn parametrize_marks_divergent_recordings_low_confidence() {
    let sb = Sandbox::new();
    let a = sb.record(
        "task",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Read","tool_input":{"file_path":"/a.rs"},"tool_response":{}}"#,
        ],
    );
    let b = sb.record(
        "task",
        &[r#"{"tool_name":"Glob","tool_input":{"pattern":"*.rs"},"tool_response":{}}"#],
    );

    assert!(sb.run(&["parametrize", &a, &b, "--emit"]).status.success());
    let skill = sb.skill_md("galdr-task-param");
    assert!(skill.contains("LOW-CONFIDENCE"));
    assert!(skill.contains("## Alignment notes"));
}

#[test]
fn distill_auto_falls_back_to_a_complete_skill_without_an_engine() {
    let sb = Sandbox::new();
    let id = sb.record("auto demo", &[BASH_STATUS]);

    // No MLX server and no Python mlx_lm: --auto must fall back to a usable, complete
    // skill (not a dead-end draft) and exit 0.
    let auto = sb.run(&["distill", &id, "--auto"]);
    assert!(
        auto.status.success(),
        "--auto must exit 0 even with no engine"
    );
    let skill = sb.skill_md("galdr-auto-demo");
    assert!(skill.contains("galdr-auto-demo"));
    // The fallback is complete: the open-standard anatomy, no draft markers.
    assert!(skill.contains("## When to use"));
    assert!(skill.contains("## Verification"));
    assert!(!skill.contains("[galdr DRAFT]"));
}

#[test]
fn diff_reports_constants_and_parameters() {
    let sb = Sandbox::new();
    let write = |path: &str| {
        format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{path}"}},"tool_response":{{}}}}"#
        )
    };
    let a = sb.record("ship", &[BASH_STATUS, &write("/repo-a/out.md")]);
    let b = sb.record("ship", &[BASH_STATUS, &write("/repo-b/out.md")]);

    let diff = sb.run(&["diff", &a, &b]);
    assert!(diff.status.success());
    let report = stdout(&diff);
    assert!(report.contains("confidence: HIGH"));
    assert!(report.contains("OUT"), "the output path is a parameter");
    assert!(report.contains("Constants:"));
}

#[test]
fn suggest_surfaces_repeated_tasks_and_dedupes_distilled_ones() {
    let sb = Sandbox::new();
    let write = |path: &str| {
        format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{path}"}},"tool_response":{{}}}}"#
        )
    };
    // Two runs of the same shape (Bash + a Write), different paths, plus a distinct
    // one-off that must not be reported as repeated.
    let a = sb.record("ship", &[BASH_STATUS, &write("/repo-a/out.md")]);
    let _b = sb.record("ship", &[BASH_STATUS, &write("/repo-b/out.md")]);
    let _c = sb.record("oneoff", &[BASH_STATUS]);

    let parse = |args: &[&str]| -> serde_json::Value {
        let out = sb.run(args);
        assert!(out.status.success(), "{args:?} failed");
        serde_json::from_str(&stdout(&out))
            .unwrap_or_else(|e| panic!("{args:?} did not emit valid JSON: {e}"))
    };

    // At the default threshold (2) only the repeated shape surfaces, counted twice.
    let opps = parse(&["suggest", "--json"]);
    let arr = opps.as_array().expect("an array of opportunities");
    assert_eq!(arr.len(), 1, "only the repeated shape is an opportunity");
    assert_eq!(arr[0]["count"], 2);
    let recs = arr[0]["recordings"].as_array().unwrap();
    assert_eq!(recs.len(), 2);

    // Distilling one run of the shape dedupes it out: nothing repeated remains.
    assert!(sb.run(&["distill", &a]).status.success());
    let after = parse(&["suggest", "--json"]);
    assert!(
        after.as_array().unwrap().is_empty(),
        "an installed skill dedupes its shape out of the opportunities"
    );
}

#[test]
fn suggest_and_bench_render_human_reports() {
    let sb = Sandbox::new();
    // Empty install: both report nothing to do, in words, without panicking.
    assert!(sb.run(&["suggest"]).status.success());
    assert!(stdout(&sb.run(&["suggest"])).contains("No repeated"));
    assert!(stdout(&sb.run(&["bench"])).contains("No replay outcomes"));

    // A repeated shape surfaces in the human suggest table.
    let write = |path: &str| {
        format!(
            r#"{{"tool_name":"Write","tool_input":{{"file_path":"{path}"}},"tool_response":{{}}}}"#
        )
    };
    let id = sb.record("ship", &[BASH_STATUS, &write("/a/out.md")]);
    sb.record("ship", &[BASH_STATUS, &write("/b/out.md")]);
    let suggest = stdout(&sb.run(&["suggest"]));
    assert!(suggest.contains("Skill opportunities"), "{suggest}");
    assert!(suggest.contains("galdr distill"), "{suggest}");

    // A recorded outcome surfaces in the human bench table.
    let refined = sb.home().join("s.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-ship\ndescription: \"ship\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    assert!(
        sb.cmd()
            .args(["distill", &id, "--from"])
            .arg(&refined)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        sb.run(&[
            "outcome",
            "usage",
            "--skill",
            "galdr-ship",
            "--rec",
            &id,
            "--outcome",
            "success",
        ])
        .status
        .success()
    );
    let bench = stdout(&sb.run(&["bench"]));
    assert!(bench.contains("Replay reliability"), "{bench}");
    assert!(bench.contains("galdr-ship"), "{bench}");
}

#[test]
fn rec_control_rejects_double_start_and_orphan_stop() {
    let sb = Sandbox::new();
    // Stopping with nothing active is an error, not a silent success.
    let stop = sb.run(&["rec", "stop"]);
    assert!(!stop.status.success());
    assert!(stderr(&stop).contains("no active recording"));

    // Status with nothing active reports it cleanly (exit 0).
    let status = sb.run(&["rec", "status"]);
    assert!(status.status.success());
    assert!(stdout(&status).contains("no active recording"));

    // First start succeeds; a second start while one is active is refused.
    assert!(sb.run(&["rec", "start", "one"]).status.success());
    let again = sb.run(&["rec", "start", "two"]);
    assert!(!again.status.success());
    assert!(stderr(&again).contains("already active"));

    // Stop succeeds and a second stop is an orphan error again.
    assert!(sb.run(&["rec", "stop"]).status.success());
    assert!(!sb.run(&["rec", "stop"]).status.success());
}

#[test]
fn a_corrupt_active_flag_is_treated_as_not_recording() {
    let sb = Sandbox::new();
    // Establish ~/.galdr by starting and stopping once.
    assert!(sb.run(&["rec", "start", "seed"]).status.success());
    assert!(sb.run(&["rec", "stop"]).status.success());
    // Corrupt the active flag: the sensor and control must treat it as "no recording"
    // (the safe side), not crash.
    std::fs::write(sb.home().join(".galdr/active"), "}{ not json").unwrap();
    let status = sb.run(&["rec", "status"]);
    assert!(status.status.success());
    assert!(stdout(&status).contains("no active recording"));
    // And a fresh start works despite the garbage flag.
    assert!(sb.run(&["rec", "start", "fresh"]).status.success());
}

#[test]
fn list_is_empty_on_a_fresh_install() {
    let sb = Sandbox::new();
    let out = sb.run(&["list"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("no recordings yet"));
}

#[test]
fn bench_reports_replay_hit_rate_from_recorded_outcomes() {
    let sb = Sandbox::new();
    let id = sb.record("bench", &[BASH_STATUS]);
    let refined = sb.home().join("bench.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-bench\ndescription: \"bench task\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    assert!(
        sb.cmd()
            .args(["distill", &id, "--from"])
            .arg(&refined)
            .output()
            .unwrap()
            .status
            .success()
    );

    // Two recorded replays of the skill: one clean, one failed after a retry.
    let record_outcome = |outcome: &str, retries: &str| {
        sb.run(&[
            "outcome",
            "usage",
            "--skill",
            "galdr-bench",
            "--rec",
            &id,
            "--outcome",
            outcome,
            "--retries",
            retries,
        ])
    };
    assert!(record_outcome("success", "0").status.success());
    assert!(record_outcome("failed", "1").status.success());

    let out = sb.run(&["bench", "--json"]);
    assert!(out.status.success());
    let report: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(report["total_replays"], 2);
    assert_eq!(report["overall_success_rate"], 0.5);
    let skill = report["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["skill_name"] == "galdr-bench")
        .expect("galdr-bench in the report");
    assert_eq!(skill["uses"], 2);
    assert_eq!(skill["success"], 1);
    assert_eq!(skill["failed"], 1);
    assert_eq!(skill["success_rate"], 0.5);
    assert_eq!(skill["avg_retries"], 0.5);
}

#[test]
fn rec_status_and_capture_policy_work() {
    let sb = Sandbox::new();
    assert!(stdout(&sb.run(&["rec", "status"])).contains("no active recording"));

    std::fs::create_dir_all(sb.home().join(".galdr")).unwrap();
    std::fs::write(
        sb.home().join(".galdr/config.json"),
        r#"{"capture":{"deny_tools":["Secret"],"deny_cwd_prefixes":["/private"],"max_response_chars":12}}"#,
    )
    .unwrap();

    assert!(sb.run(&["rec", "start", "capture"]).status.success());
    let id = sb.active_rec_id();
    assert!(
        sb.hook(
            r#"{"tool_name":"Secret","tool_input":{"value":"x"},"tool_response":{"token":"abc"}}"#,
            false,
        )
        .status
        .success()
    );
    assert_eq!(sb.span_lines(&id), 0, "denied tools are not recorded");

    assert!(
        sb.hook(
            r#"{"tool_name":"Bash","tool_input":{"command":"echo hi"},"tool_response":{"stdout":"abcdefghijklmnopqrstuvwxyz"},"cwd":"/tmp"}"#,
            false,
        )
        .status
        .success()
    );
    assert_eq!(sb.span_lines(&id), 1);
    let span = std::fs::read_to_string(sb.home().join(".galdr/spans").join(format!("{id}.jsonl")))
        .unwrap();
    assert!(span.contains("galdr_truncated"));

    let status = stdout(&sb.run(&["rec", "status"]));
    assert!(status.contains("active recording: capture"));
    assert!(status.contains("steps: 1"));
}

#[test]
fn skills_catalog_reports_status_readiness_and_delta() {
    let sb = Sandbox::new();
    let id = sb.record("readiness", &[BASH_STATUS]);

    // The agent-assisted scaffolding path still exists behind --draft and scores
    // lower (draft markers, missing refinement) before an agent finishes it.
    assert!(sb.run(&["distill", &id, "--draft"]).status.success());
    let draft_listing = stdout(&sb.run(&["skills"]));
    assert!(draft_listing.contains("galdr-readiness"));
    assert!(draft_listing.contains("draft"));
    assert!(draft_listing.contains("readiness"));

    let refined = sb.home().join("refined.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-readiness\ndescription: \"readiness check\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let install = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .output()
        .unwrap();
    assert!(install.status.success());

    let final_listing = stdout(&sb.run(&["skills"]));
    assert!(final_listing.contains("final"));
    assert!(
        final_listing.contains("(+"),
        "readiness delta should show the final skill improved: {final_listing}"
    );

    let evaluations = stdout(&sb.run(&["evaluations", "--skill", "galdr-readiness"]));
    assert!(evaluations.contains("readiness_lint"));
    assert!(evaluations.contains("galdr-readiness"));
}

#[test]
fn outcome_usage_and_labels_survive_reindex() {
    let sb = Sandbox::new();
    let id = sb.record("outcome", &[BASH_STATUS]);
    let refined = sb.home().join("outcome.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: galdr-outcome\ndescription: \"outcome capture\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let install = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .output()
        .unwrap();
    assert!(install.status.success());

    let usage = sb.run(&[
        "outcome",
        "usage",
        "--skill",
        "galdr-outcome",
        "--rec",
        &id,
        "--task-kind",
        "smoke",
        "--outcome",
        "success",
        "--retries",
        "1",
        "--manual-interventions",
        "2",
        "--notes",
        "worked after one retry",
    ]);
    assert!(usage.status.success());
    assert!(stdout(&usage).contains("usage recorded"));

    let label = sb.run(&[
        "outcome",
        "label",
        "--skill",
        "galdr-outcome",
        "--rec",
        &id,
        "--evaluator",
        "human",
        "--label",
        "accepted",
        "--confidence",
        "0.9",
        "--notes",
        "reviewed",
    ]);
    assert!(label.status.success());
    assert!(stdout(&label).contains("outcome recorded"));

    let usage_log = sb.home().join(".galdr/outcomes/skill_usage.jsonl");
    let outcome_log = sb.home().join(".galdr/outcomes/skill_outcomes.jsonl");
    assert!(
        std::fs::read_to_string(usage_log)
            .unwrap()
            .contains("success")
    );
    assert!(
        std::fs::read_to_string(outcome_log)
            .unwrap()
            .contains("accepted")
    );

    assert!(sb.run(&["reindex"]).status.success());
    let listing = stdout(&sb.run(&["outcome", "list", "--skill", "galdr-outcome"]));
    assert!(listing.contains("success"));
    assert!(listing.contains("accepted"));
    assert!(listing.contains("interventions  2"));
}

#[test]
fn distill_from_rejects_unfinished_skills() {
    let sb = Sandbox::new();
    let id = sb.record("unfinished", &[BASH_STATUS]);
    let bad = sb.home().join("bad.md");
    std::fs::write(
        &bad,
        "---\nname: galdr-unfinished\ndescription: \"bad\"\n---\n\n## Goal\nx\n## Procedure\ny\n",
    )
    .unwrap();
    let install = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!install.status.success());
    assert!(String::from_utf8_lossy(&install.stderr).contains("Success criteria"));
}

#[test]
fn setup_claude_check_and_print_work_without_mutating_settings() {
    let sb = Sandbox::new();
    let missing = stdout(&sb.run(&["setup", "claude", "--check"]));
    assert!(missing.contains("settings not found"));

    let snippet = stdout(&sb.run(&["setup", "claude", "--print"]));
    assert!(snippet.contains("PostToolUse"));
    assert!(snippet.contains("galdr hook"));

    let settings = sb.home().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(&settings, snippet).unwrap();
    let configured = stdout(&sb.run(&["setup", "claude", "--check"]));
    assert!(configured.contains("is configured"));
}

#[test]
fn export_omits_raw_by_default_and_can_write_redacted_raw() {
    let sb = Sandbox::new();
    let id = sb.record(
        "export",
        &[r#"{"tool_name":"Bash","tool_input":{"command":"deploy","api_key":"secret-key"},"tool_response":{"token":"secret-token","ok":true}}"#],
    );

    let out = sb.home().join("export-default");
    let export = sb
        .cmd()
        .args(["export", &id, "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(export.status.success());
    assert!(out.join("recording.json").exists());
    assert!(out.join("steps.md").exists());
    assert!(out.join("skills.json").exists());
    assert!(out.join("usage.json").exists());
    assert!(out.join("outcomes.json").exists());
    assert!(!out.join("raw.jsonl").exists());

    let redacted = sb.home().join("export-redacted");
    let export = sb
        .cmd()
        .args(["export", &id, "--out"])
        .arg(&redacted)
        .arg("--redact")
        .output()
        .unwrap();
    assert!(export.status.success());
    let raw = std::fs::read_to_string(redacted.join("raw.redacted.jsonl")).unwrap();
    assert!(raw.contains("[REDACTED]"));
    assert!(!raw.contains("secret-token"));
    assert!(!raw.contains("secret-key"));
}

#[test]
fn doctor_passes_when_claude_hook_is_configured() {
    let sb = Sandbox::new();
    let settings = sb.home().join(".claude/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    std::fs::write(
        &settings,
        r#"{"hooks":{"PostToolUse":[{"hooks":[{"type":"command","command":"galdr hook"}]}]}}"#,
    )
    .unwrap();
    let doctor = sb.run(&["doctor"]);
    assert!(
        doctor.status.success(),
        "{}\n{}",
        stdout(&doctor),
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert!(stdout(&doctor).contains("doctor: ok"));
}

/// Optional daemon round-trip. Kept robust (generous polling, guaranteed
/// teardown) but isolated to its own temp HOME so it never disturbs the others.
#[test]
fn daemon_indexes_and_answers_queries() {
    let sb = Sandbox::new();
    let pidfile = sb.home().join(".galdr/galdrd.pid");
    let socket = sb.home().join(".galdr/galdrd.sock");

    assert!(stdout(&sb.run(&["daemon", "status"])).contains("daemon stopped"));
    assert!(sb.run(&["daemon", "--detach"]).status.success());

    // A guard that kills the daemon on the way out, even if an assert fails.
    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(pid) = std::fs::read_to_string(&self.0)
                && let Ok(pid) = pid.trim().parse::<i32>()
            {
                let _ = Command::new("kill")
                    .arg(pid.to_string())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
    let _guard = Guard(pidfile.clone());

    // Wait for the socket to appear (up to ~5s).
    let mut ready = false;
    for _ in 0..100 {
        if socket.exists() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ready, "daemon socket never appeared");
    assert!(stdout(&sb.run(&["daemon", "status"])).contains("daemon running"));

    sb.record("daemon demo", &[BASH_STATUS]);
    // Give the close notification a moment to be indexed.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let list = sb.run(&["list"]);
    assert!(list.status.success());
    assert!(
        stdout(&list).contains("daemon demo"),
        "the daemon-backed catalog should list the recording"
    );

    let stop = sb.run(&["daemon", "stop"]);
    assert!(stop.status.success());
    assert!(stdout(&stop).contains("daemon stopped"));
}
