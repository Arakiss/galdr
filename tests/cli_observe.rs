mod common;

use common::*;

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
fn human_events_reindex_show_distill_and_export() {
    let sb = Sandbox::new();
    write_human_recording(
        &sb,
        "01HUMAN",
        "human issue form",
        serde_json::json!({
            "policy": "redacted",
            "kind": "text",
            "chars": 24
        }),
    );

    let reindex = sb.run(&["reindex"]);
    assert!(reindex.status.success(), "{}", stderr(&reindex));

    let show = sb.run(&["show", "01HUMAN", "--json"]);
    assert!(show.status.success(), "{}", stderr(&show));
    let detail: serde_json::Value = serde_json::from_str(&stdout(&show)).unwrap();
    assert_eq!(detail["steps"][0]["event_kind"], "human");
    assert_eq!(
        detail["steps"][0]["summary"],
        "type into \"Issue title\" (text, 24 chars)"
    );

    let distill = sb.run(&["distill", "01HUMAN", "--fast"]);
    assert!(distill.status.success(), "{}", stderr(&distill));
    let skill = sb.skill_md("galdr-human-issue-form");
    assert!(skill.contains("browser workflow"), "{skill}");
    assert!(skill.contains("type into \"Issue title\""), "{skill}");
    assert!(skill.contains("Confirm the issue was saved."), "{skill}");

    write_human_recording(
        &sb,
        "01HUMANLIT",
        "human literal",
        serde_json::json!({
            "policy": "literal",
            "value": "Visible customer escalation"
        }),
    );
    let literal_distill = sb.run(&["distill", "01HUMANLIT", "--fast"]);
    assert!(
        literal_distill.status.success(),
        "{}",
        stderr(&literal_distill)
    );
    let literal_skill = sb.skill_md("galdr-human-literal");
    assert!(
        !literal_skill.contains("Visible customer escalation"),
        "literal typed text leaked into skill:\n{literal_skill}"
    );
    assert!(
        literal_skill
            .contains("- `<Issue title>` — text for Issue title at step 1 (27 chars recorded)"),
        "{literal_skill}"
    );

    let out = sb.home().join("human-export");
    let export = sb
        .cmd()
        .args(["export", "01HUMANLIT", "--out"])
        .arg(&out)
        .arg("--redact")
        .output()
        .unwrap();
    assert!(export.status.success(), "{}", stderr(&export));
    let raw = std::fs::read_to_string(out.join("raw.redacted.jsonl")).unwrap();
    assert!(!raw.contains("Visible customer escalation"), "{raw}");
    assert!(raw.contains(r#""policy":"redacted""#), "{raw}");
    assert!(raw.contains(r#""kind":"literal""#), "{raw}");
}

#[test]
fn judge_import_show_summary_distill_and_reindex() {
    let sb = Sandbox::new();
    let first = sb.record(
        "on policy task",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Write","tool_input":{"file_path":"/repo/out.txt"},"tool_response":{}}"#,
        ],
    );
    let second = sb.record(
        "on policy task",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Write","tool_input":{"file_path":"/repo/other.txt"},"tool_response":{}}"#,
        ],
    );
    let input = sb.home().join("judgments.json");
    std::fs::write(
        &input,
        serde_json::json!({
            "task_key": "on-policy-demo",
            "judge": "external-strong-judge",
            "recordings": [
                {
                    "rec_id": first,
                    "judgments": [
                        {
                            "step": 2,
                            "verdict": "fork",
                            "rationale": "executor wrote the file before checking the expected output",
                            "suggested_action": "inspect the expected output before writing"
                        }
                    ]
                },
                {
                    "rec_id": second,
                    "judgments": [
                        {
                            "step": 2,
                            "verdict": "ok",
                            "rationale": "executor checked the target before writing"
                        }
                    ]
                }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let imported = sb
        .cmd()
        .args(["judge", "import", "--file"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(imported.status.success(), "{}", stderr(&imported));
    assert!(stdout(&imported).contains("judgments imported"));

    let shown = stdout(&sb.run(&["show", &first]));
    assert!(shown.contains("judgment fork"), "{shown}");
    assert!(
        shown.contains("inspect the expected output before writing"),
        "{shown}"
    );

    let shown_json = sb.run(&["show", &first, "--json"]);
    assert!(shown_json.status.success(), "{}", stderr(&shown_json));
    let detail: serde_json::Value = serde_json::from_str(&stdout(&shown_json)).unwrap();
    assert_eq!(
        detail["steps"][1]["judgments"][0]["verdict"],
        serde_json::json!("fork")
    );

    let summary = stdout(&sb.run(&["judge", "summary", "--task", "on-policy-demo"]));
    assert!(
        summary.contains("step 2")
            && (summary.contains("1/2 forked") || summary.contains("1/2 fork(s)")),
        "{summary}"
    );

    assert!(sb.run(&["reindex"]).status.success());
    let after_reindex = stdout(&sb.run(&["show", &first]));
    assert!(after_reindex.contains("judgment fork"), "{after_reindex}");

    let distill = sb.run(&["distill", &first, "--fast"]);
    assert!(distill.status.success(), "{}", stderr(&distill));
    let skill = sb.skill_md("galdr-on-policy-task");
    assert!(skill.contains("## Known Forking Points"), "{skill}");
    assert!(skill.contains("measured external judgments"), "{skill}");
    assert!(
        skill.contains("Do this instead: inspect the expected output before writing"),
        "{skill}"
    );
}

#[test]
fn observe_synthetic_records_a_human_trace() {
    let sb = Sandbox::new();
    let observed = sb.run(&[
        "observe",
        "synthetic",
        "synthetic human form",
        "--fixture",
        "browser-form",
    ]);
    assert!(observed.status.success(), "{}", stderr(&observed));
    let said = stdout(&observed);
    assert!(said.contains("observed \"synthetic human form\""), "{said}");

    let list = stdout(&sb.run(&["list"]));
    assert!(list.contains("synthetic human form"), "{list}");
    assert!(list.contains("4 steps"), "{list}");

    let show = sb.run(&["show", "synthetic human form", "--json"]);
    assert!(show.status.success(), "{}", stderr(&show));
    let detail: serde_json::Value = serde_json::from_str(&stdout(&show)).unwrap();
    assert_eq!(detail["recording"]["name"], "synthetic human form");
    assert_eq!(detail["steps"].as_array().unwrap().len(), 4);
    assert!(
        detail["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["event_kind"] == "human")
    );
    assert_eq!(
        detail["steps"][0]["summary"],
        "navigate https://example.test/issues/new"
    );
    assert_eq!(
        detail["steps"][1]["summary"],
        "type into \"Issue title\" (text, 24 chars)"
    );
    assert_eq!(
        detail["steps"][2]["summary"],
        "select \"Priority\" = \"High\""
    );
    assert_eq!(
        detail["steps"][3]["summary"],
        "click button \"Create issue\""
    );

    let distill = sb.run(&["distill", "synthetic human form", "--fast"]);
    assert!(distill.status.success(), "{}", stderr(&distill));
    let skill = sb.skill_md("galdr-synthetic-human-form");
    assert!(skill.contains("browser workflow"), "{skill}");
    assert!(
        skill.contains("navigate https://example.test/issues/new"),
        "{skill}"
    );
    assert!(skill.contains("select \"Priority\" = \"High\""), "{skill}");
    assert!(
        skill.contains("Confirm the created issue page is open or a success message appears."),
        "{skill}"
    );
}

#[test]
fn observe_browser_collector_records_loopback_events() {
    let sb = Sandbox::new();
    let start = sb.run(&[
        "observe",
        "browser",
        "start",
        "browser collector",
        "--url",
        "https://example.test/form",
        "--no-open",
    ]);
    assert!(start.status.success(), "{}", stderr(&start));
    let port = active_browser_port(&sb);

    post_browser_event(
        port,
        serde_json::json!({
            "ts": "2026-06-30T00:00:00Z",
            "action": "human.browser.navigate",
            "source": {
                "kind": "browser",
                "url": "https://example.test/form",
                "title": "Demo form"
            }
        }),
    );
    post_browser_event(
        port,
        serde_json::json!({
            "ts": "2026-06-30T00:00:01Z",
            "action": "human.browser.input",
            "source": {
                "kind": "browser",
                "url": "https://example.test/form",
                "title": "Demo form"
            },
            "target": {
                "primary": {
                    "kind": "label",
                    "value": "Email"
                },
                "label": "Email"
            },
            "value": {
                "policy": "redacted",
                "kind": "text",
                "chars": 16
            }
        }),
    );

    let status = stdout(&sb.run(&["observe", "browser", "status"]));
    assert!(status.contains("events: 2"), "{status}");
    assert!(status.contains("server: up"), "{status}");

    let stop = sb.run(&["observe", "browser", "stop"]);
    assert!(stop.status.success(), "{}", stderr(&stop));
    let said = stdout(&stop);
    assert!(said.contains("stopped browser observation"), "{said}");
    assert!(said.contains("2 human steps"), "{said}");

    let show = sb.run(&["show", "browser collector", "--json"]);
    assert!(show.status.success(), "{}", stderr(&show));
    let detail: serde_json::Value = serde_json::from_str(&stdout(&show)).unwrap();
    assert_eq!(detail["steps"].as_array().unwrap().len(), 2);
    assert_eq!(detail["steps"][0]["event_kind"], "human");
    assert_eq!(
        detail["steps"][0]["summary"],
        "navigate https://example.test/form"
    );
    assert_eq!(
        detail["steps"][1]["summary"],
        "type into \"Email\" (text, 16 chars)"
    );
}
