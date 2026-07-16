mod common;

use common::*;

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
    assert!(status.contains("1 active recording"), "{status}");
    assert!(status.contains("capture"), "{status}");
    assert!(status.contains("steps: 1"), "{status}");
}

#[test]
fn skills_catalog_reports_status_readiness_and_delta() {
    let sb = Sandbox::new();
    let id = sb.record("readiness", &[BASH_STATUS]);

    // The default distill writes a faithful draft (status draft) until an agent
    // authors it and installs the final version with --from.
    assert!(sb.run(&["distill", &id]).status.success());
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
fn regression_base_cases_pin_skill_hash_and_surface_in_validate() {
    let sb = Sandbox::new();
    let id = sb.record("regress", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &id, "--fast"]).status.success());

    let pinned = sb.run(&[
        "regress",
        "pin",
        "--skill",
        "galdr-regress",
        "--rec",
        &id,
        "--case",
        "happy path",
    ]);
    assert!(pinned.status.success(), "{}", stderr(&pinned));
    assert!(stdout(&pinned).contains("base case pinned"));

    let status = stdout(&sb.run(&["regress", "status", "--skill", "galdr-regress"]));
    assert!(
        status.contains("regression guard") || status.contains("Regression base cases"),
        "{status}"
    );
    assert!(status.contains("happy path"), "{status}");
    assert!(
        status.contains("does not execute natural-language replays")
            || status.contains("does not execute natural-language skills")
            || status.contains("pins the base cases"),
        "{status}"
    );

    let validate = sb.run(&["validate", "galdr-regress", "--json"]);
    assert!(validate.status.success(), "{}", stderr(&validate));
    let parsed: serde_json::Value = serde_json::from_str(&stdout(&validate)).unwrap();
    assert_eq!(
        parsed[0]["regression"]["base_cases"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let skill_path = sb.home().join(".agents/skills/galdr-regress/SKILL.md");
    let mut skill = std::fs::read_to_string(&skill_path).unwrap();
    skill.push_str("\n<!-- operator edit -->\n");
    std::fs::write(&skill_path, skill).unwrap();

    let changed = stdout(&sb.run(&["regress", "status", "--skill", "galdr-regress"]));
    assert!(
        changed.contains("changed_unverified") || changed.contains("1 changed/unverified"),
        "{changed}"
    );
}
