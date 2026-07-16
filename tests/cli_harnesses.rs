mod common;

use common::*;

#[test]
fn setup_codex_check_and_print_work() {
    let sb = Sandbox::new();
    let missing = stdout(&sb.run(&["setup", "codex", "--check"]));
    assert!(missing.contains("not found"));

    let snippet = stdout(&sb.run(&["setup", "codex", "--print"]));
    assert!(snippet.contains("PostToolUse"));
    assert!(snippet.contains("galdr hook"));
    // Codex skips an untrusted hook, so `--print` must spell out the trust step.
    assert!(
        snippet.contains("/hooks"),
        "print must explain trusting the hook"
    );

    let hooks = sb.home().join(".codex/hooks.json");
    std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    std::fs::write(
        &hooks,
        r#"{"hooks":{"PostToolUse":[{"matcher":".*","hooks":[{"type":"command","command":"galdr hook"}]}]}}"#,
    )
    .unwrap();
    let configured = stdout(&sb.run(&["setup", "codex", "--check"]));
    assert!(configured.contains("is present"));
}

#[test]
fn setup_cursor_check_and_print_work() {
    let sb = Sandbox::new();
    let missing = stdout(&sb.run(&["setup", "cursor", "--check"]));
    assert!(missing.contains("not found"));

    let snippet = stdout(&sb.run(&["setup", "cursor", "--print"]));
    assert!(snippet.contains("postToolUse"));
    assert!(snippet.contains("galdr hook"));

    let hooks = sb.home().join(".cursor/hooks.json");
    std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    std::fs::write(&hooks, snippet).unwrap();
    let configured = stdout(&sb.run(&["setup", "cursor", "--check"]));
    assert!(configured.contains("is present"));
}

#[test]
fn a_cursor_event_records_with_mapped_fields() {
    // Cursor's postToolUse payload renames two fields: `tool_output` (a JSON-stringified
    // string) for the result, and `conversation_id` for the session. The sensor maps both.
    let sb = Sandbox::new();
    let cursor_event = r#"{"tool_name":"Bash","tool_input":{"command":"ls"},"tool_output":"{\"exit_code\":0}","conversation_id":"conv-abc","cwd":"/x"}"#;
    let id = sb.record("cursor task", &[cursor_event]);
    assert_eq!(sb.span_lines(&id), 1);
    let span = std::fs::read_to_string(sb.home().join(".galdr/spans").join(format!("{id}.jsonl")))
        .unwrap();
    assert!(
        span.contains(r#""exit_code":0"#),
        "tool_output is parsed into tool_response: {span}"
    );
    assert!(
        span.contains("conv-abc"),
        "conversation_id maps to session_id: {span}"
    );
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
fn rm_retires_a_skill_unlinking_and_moving_it() {
    // The full lifecycle: `galdr rm` unlinks the skill from every harness, moves its
    // directory into .retired (never a hard delete), and refreshes the catalog so it
    // drops out of `galdr skills`.
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.home().join(".claude/skills")).unwrap();

    let id = sb.record("retire me", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &id, "--fast"]).status.success());

    let link = sb.home().join(".claude/skills/galdr-retire-me");
    assert!(link.exists(), "the skill should be linked into the harness");
    assert!(stdout(&sb.run(&["skills"])).contains("galdr-retire-me"));

    let rm = sb.run(&["rm", "galdr-retire-me", "--force"]);
    assert!(rm.status.success(), "{}", stderr(&rm));
    let said = stdout(&rm);
    assert!(
        said.contains("Unlinked"),
        "reports what it unlinked: {said}"
    );
    assert!(said.contains("retired 'galdr-retire-me'"), "{said}");

    // The harness symlink is gone (and does not dangle).
    assert!(
        sb.home()
            .join(".claude/skills/galdr-retire-me")
            .symlink_metadata()
            .is_err(),
        "the harness symlink must be removed"
    );
    // The canonical directory left the active root for .retired.
    assert!(!sb.home().join(".agents/skills/galdr-retire-me").exists());
    assert!(
        sb.home()
            .join(".agents/skills/.retired/galdr-retire-me/SKILL.md")
            .exists(),
        "the skill should be moved to .retired, not deleted"
    );
    // The refreshed catalog no longer lists it.
    let listing = stdout(&sb.run(&["skills"]));
    assert!(
        !listing.contains("galdr-retire-me"),
        "a retired skill must drop out of the catalog: {listing}"
    );
}

#[test]
fn rm_of_a_missing_skill_fails_clearly() {
    let sb = Sandbox::new();
    let out = sb.run(&["rm", "does-not-exist", "--force"]);
    assert!(!out.status.success(), "removing a missing skill must fail");
    assert!(stderr(&out).contains("not installed"), "{}", stderr(&out));
}

#[test]
fn rm_without_force_refuses_in_a_non_interactive_context() {
    // Tests run with no TTY. Without --force the retire must refuse (a serious CLI does
    // not mutate on a non-interactive run without an explicit opt-in) and change nothing.
    let sb = Sandbox::new();
    let id = sb.record("keep unless forced", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &id, "--fast"]).status.success());

    let out = sb.run(&["rm", "galdr-keep-unless-forced"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--force"), "{}", stderr(&out));
    assert!(
        sb.home()
            .join(".agents/skills/galdr-keep-unless-forced/SKILL.md")
            .exists(),
        "a refused retire must leave the skill in place"
    );
}

#[test]
fn rm_suffixes_a_retired_name_that_already_exists() {
    // Retiring two skills that share a name must not clobber the earlier copy: the
    // second lands on a numeric suffix under .retired.
    let sb = Sandbox::new();
    let first = sb.record("twice", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &first, "--fast"]).status.success());
    assert!(sb.run(&["rm", "galdr-twice", "--force"]).status.success());

    let second = sb.record("twice", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &second, "--fast"]).status.success());
    assert!(sb.run(&["rm", "galdr-twice", "--force"]).status.success());

    assert!(
        sb.home()
            .join(".agents/skills/.retired/galdr-twice/SKILL.md")
            .exists()
    );
    assert!(
        sb.home()
            .join(".agents/skills/.retired/galdr-twice.1/SKILL.md")
            .exists(),
        "the second retirement of the same name must be suffixed"
    );
}
