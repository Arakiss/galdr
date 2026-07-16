mod common;

use common::*;

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
fn default_distill_writes_an_unauthored_draft() {
    // A replay of the tool calls is not yet a skill: the default hands the agent a
    // faithful draft to author — status draft, real steps, an authoring marker — and
    // prints the brief that tells the agent what to add and how to install it.
    let sb = Sandbox::new();
    let id = sb.record(
        "deploy preview",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Write","tool_input":{"file_path":"/repo/out.txt"},"tool_response":{}}"#,
        ],
    );
    let out = sb.run(&["distill", &id]);
    assert!(out.status.success());
    let said = stdout(&out);
    assert!(said.contains("author the real skill"), "{said}");
    assert!(said.contains("galdr distill --from"), "{said}");

    let skill = sb.skill_md("galdr-deploy-preview");
    assert!(skill.contains("galdr:unauthored"), "draft marker:\n{skill}");
    for section in ["## When to use", "## Steps", "## Verification"] {
        assert!(skill.contains(section), "missing {section}:\n{skill}");
    }
    // It is catalogued as a draft, pending authoring — not final.
    let listing = stdout(&sb.run(&["skills"]));
    assert!(
        listing.contains("draft"),
        "should list as draft:\n{listing}"
    );
}

#[test]
fn fast_distill_produces_a_complete_usable_skill() {
    // `--fast` is the mechanical one-shot: a complete, valid skill in the open-standard
    // anatomy, installed as final — no authoring pass, no draft.
    let sb = Sandbox::new();
    let id = sb.record(
        "deploy preview",
        &[
            BASH_STATUS,
            r#"{"tool_name":"Write","tool_input":{"file_path":"/repo/out.txt"},"tool_response":{}}"#,
        ],
    );
    assert!(sb.run(&["distill", &id, "--fast"]).status.success());

    let skill = sb.skill_md("galdr-deploy-preview");
    for section in ["## When to use", "## Inputs", "## Steps", "## Verification"] {
        assert!(skill.contains(section), "missing {section}:\n{skill}");
    }
    assert!(!skill.contains("galdr:unauthored"));
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
        sb.run(&["distill", &id, "--fast", "--name", "rust-greenlight"])
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
fn distill_from_honors_the_frontmatter_name() {
    // A refined skill renamed in its frontmatter must install under that name, not the
    // mechanical recording slug — so `galdr validate <name>` finds it and identity does
    // not split between the directory and the skill's own `name:`.
    let sb = Sandbox::new();
    let id = sb.record("visual branding upgrade", &[BASH_STATUS]);

    let refined = sb.home().join("refined.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: skill-family-upgrade\ndescription: \"upgrade a family of related skills\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();

    let out = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains(
            "installed as skill-family-upgrade (renamed from recording slug galdr-visual-branding-upgrade)"
        ),
        "{}",
        stdout(&out)
    );

    // Installed under the frontmatter name; the mechanical slug directory is not created.
    assert!(
        sb.home()
            .join(".agents/skills/skill-family-upgrade/SKILL.md")
            .exists()
    );
    assert!(
        !sb.home()
            .join(".agents/skills/galdr-visual-branding-upgrade")
            .exists(),
        "the recording-slug directory must not be created"
    );

    // The catalog keys it under the frontmatter name, and `validate` resolves it.
    assert!(stdout(&sb.run(&["skills"])).contains("skill-family-upgrade"));
    assert!(
        sb.run(&["validate", "skill-family-upgrade"])
            .status
            .success(),
        "the renamed skill must validate by its frontmatter name"
    );
}

#[test]
fn distill_from_name_flag_overrides_the_frontmatter() {
    // An explicit --name keeps priority over the frontmatter name.
    let sb = Sandbox::new();
    let id = sb.record("branding", &[BASH_STATUS]);
    let refined = sb.home().join("refined.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: frontmatter-name\ndescription: \"x\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let out = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .args(["--name", "explicit-name"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        sb.home()
            .join(".agents/skills/explicit-name/SKILL.md")
            .exists()
    );
    assert!(
        !sb.home().join(".agents/skills/frontmatter-name").exists(),
        "the frontmatter name must not win over an explicit --name"
    );
}

#[test]
fn distill_from_refuses_a_frontmatter_name_held_by_another_skill() {
    // A frontmatter name already taken by an UNRELATED skill (a different recording) is
    // refused without touching anything — no silent clobber of someone else's skill.
    let sb = Sandbox::new();
    let other = sb.record("other", &[BASH_STATUS]);
    let other_file = sb.home().join("other.md");
    std::fs::write(
        &other_file,
        format!(
            "---\nname: taken-name\ndescription: \"the incumbent\"\n---\n\n## Provenance\n- rec_id: `{other}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    assert!(
        sb.cmd()
            .args(["distill", &other, "--from"])
            .arg(&other_file)
            .output()
            .unwrap()
            .status
            .success()
    );

    let mine = sb.record("mine", &[BASH_STATUS]);
    let mine_file = sb.home().join("mine.md");
    std::fs::write(
        &mine_file,
        format!(
            "---\nname: taken-name\ndescription: \"the challenger\"\n---\n\n## Provenance\n- rec_id: `{mine}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let refused = sb
        .cmd()
        .args(["distill", &mine, "--from"])
        .arg(&mine_file)
        .output()
        .unwrap();
    assert!(
        !refused.status.success(),
        "a colliding frontmatter name must be refused"
    );
    assert!(
        stderr(&refused).contains("already exists"),
        "{}",
        stderr(&refused)
    );

    // The incumbent is untouched, and the challenger's own recording slug is not created.
    let incumbent =
        std::fs::read_to_string(sb.home().join(".agents/skills/taken-name/SKILL.md")).unwrap();
    assert!(incumbent.contains("the incumbent"), "{incumbent}");
    assert!(!sb.home().join(".agents/skills/galdr-mine").exists());
}

#[test]
fn distill_from_migrates_a_stale_draft_under_the_slug() {
    // A prior draft under the mechanical slug must not survive as a second copy once the
    // authored skill installs under its frontmatter name: it is retired to .retired.
    let sb = Sandbox::new();
    let id = sb.record("family upgrade", &[BASH_STATUS]);
    assert!(sb.run(&["distill", &id]).status.success());
    assert!(
        sb.home()
            .join(".agents/skills/galdr-family-upgrade/SKILL.md")
            .exists(),
        "the draft is written under the recording slug"
    );

    let refined = sb.home().join("refined.md");
    std::fs::write(
        &refined,
        format!(
            "---\nname: skill-family-upgrade\ndescription: \"upgrade a family of related skills\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n## Success criteria\nz\n"
        ),
    )
    .unwrap();
    let out = sb
        .cmd()
        .args(["distill", &id, "--from"])
        .arg(&refined)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));

    // The renamed skill is the only live copy; the stale draft was retired, not left behind.
    assert!(
        sb.home()
            .join(".agents/skills/skill-family-upgrade/SKILL.md")
            .exists()
    );
    assert!(
        !sb.home()
            .join(".agents/skills/galdr-family-upgrade")
            .exists(),
        "the stale slug directory must be moved aside"
    );
    assert!(
        sb.home()
            .join(".agents/skills/.retired/galdr-family-upgrade/SKILL.md")
            .exists(),
        "the stale draft should be retired to .retired"
    );
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
