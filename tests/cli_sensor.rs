mod common;

use common::*;

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
