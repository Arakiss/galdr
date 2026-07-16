mod common;

use common::*;

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
