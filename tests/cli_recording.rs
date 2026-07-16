mod common;

use common::*;

#[test]
fn bare_galdr_shows_a_friendly_overview() {
    // `galdr` with no subcommand is a human at a terminal — show a home screen, not a
    // clap usage error. Piped (not a TTY), the output is plain text with no ANSI codes.
    let sb = Sandbox::new();
    sb.record("demo-task", &[BASH_STATUS]);

    let out = sb.run(&[]);
    assert!(out.status.success(), "bare `galdr` must exit 0");
    let text = stdout(&out);
    assert!(text.contains("galdr"), "{text}");
    assert!(text.contains("next"), "shows next steps: {text}");
    assert!(text.contains("galdr rec start"), "{text}");
    assert!(
        text.contains("1 recordings"),
        "reflects the catalog: {text}"
    );
    assert!(
        !text.contains('\u{1b}'),
        "no ANSI escapes when piped: {text:?}"
    );
}

#[test]
fn recording_references_resolve_without_a_ulid() {
    // The human DX: never type a 26-char id. `distill`/`show` default to the most
    // recent recording and also accept a recording name.
    let sb = Sandbox::new();
    let older = sb.record("weekly-report", &[BASH_STATUS]);
    let newest = sb.record("ship-preview", &[BASH_STATUS]);

    // `galdr distill` with no argument distills the most recent recording.
    assert!(sb.run(&["distill"]).status.success());
    let skill = sb.skill_md("galdr-ship-preview");
    assert!(
        skill.contains(&newest),
        "the newest recording was distilled"
    );

    // `galdr show <name>` resolves by name (and is not the newest here).
    let shown = sb.run(&["show", "weekly-report", "--json"]);
    assert!(shown.status.success());
    let detail: serde_json::Value = serde_json::from_str(&stdout(&shown)).unwrap();
    assert_eq!(detail["recording"]["rec_id"], older.as_str());

    // The full id still works, and a miss fails with guidance, not a cryptic id error.
    assert!(sb.run(&["show", &newest]).status.success());
    let miss = sb.run(&["show", "does-not-exist"]);
    assert!(!miss.status.success());
    assert!(stderr(&miss).contains("galdr list"));
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

    // Concurrent recordings are allowed now: a second start also succeeds.
    assert!(sb.run(&["rec", "start", "one"]).status.success());
    assert!(sb.run(&["rec", "start", "two"]).status.success());
    let both = stdout(&sb.run(&["rec", "status"]));
    assert!(both.contains("2 active recordings"), "{both}");
    assert!(both.contains("one") && both.contains("two"), "{both}");

    // A bare stop with several active is refused, listing which to name.
    let ambiguous = sb.run(&["rec", "stop"]);
    assert!(!ambiguous.status.success());
    assert!(
        stderr(&ambiguous).contains("specify which"),
        "{}",
        stderr(&ambiguous)
    );

    // Stop by name closes exactly that one; the other stays active.
    assert!(sb.run(&["rec", "stop", "one"]).status.success());
    let after = stdout(&sb.run(&["rec", "status"]));
    assert!(after.contains("1 active recording"), "{after}");
    assert!(after.contains("two"), "{after}");

    // A bare stop now closes the sole remaining one; a further stop is an orphan error.
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
