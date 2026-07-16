mod common;

use common::*;

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
