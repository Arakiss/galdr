mod common;

use common::*;

#[test]
fn concurrent_recordings_bind_and_isolate_by_session() {
    // The core of the concurrency feature: two agent sessions record at once, each
    // span receives only its own session's steps, and stopping one leaves the other
    // recording.
    let sb = Sandbox::new();
    for name in ["projA", "projB", "projC"] {
        std::fs::create_dir_all(sb.home().join(name)).unwrap();
    }
    // Canonicalize so the event `cwd` matches `origin_cwd` exactly: on macOS the
    // recording process's getcwd() resolves /var → /private/var, so an unresolved
    // temp path would never be seen as "under" the origin.
    let proj_a = std::fs::canonicalize(sb.home().join("projA")).unwrap();
    let proj_b = std::fs::canonicalize(sb.home().join("projB")).unwrap();
    let proj_c = std::fs::canonicalize(sb.home().join("projC")).unwrap();

    // Start two recordings, each from its own directory (distinct origin_cwd).
    assert!(
        sb.cmd()
            .current_dir(&proj_a)
            .args(["rec", "start", "alpha"])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        sb.cmd()
            .current_dir(&proj_b)
            .args(["rec", "start", "beta"])
            .status()
            .unwrap()
            .success()
    );

    // A PostToolUse event bound to a session under a working directory.
    let event = |session: &str, cwd: &std::path::Path, cmd: &str| {
        format!(
            r#"{{"tool_name":"Bash","tool_input":{{"command":"{cmd}"}},"tool_response":{{}},"session_id":"{session}","cwd":"{}"}}"#,
            cwd.display()
        )
    };

    // sessA acts under projA → binds alpha; sessB under projB → binds beta. Interleaved.
    assert!(
        sb.hook(&event("sessA", &proj_a, "a-one"), false)
            .status
            .success()
    );
    assert!(
        sb.hook(&event("sessB", &proj_b, "b-one"), false)
            .status
            .success()
    );
    assert!(
        sb.hook(&event("sessB", &proj_b, "b-two"), false)
            .status
            .success()
    );
    assert!(
        sb.hook(&event("sessA", &proj_a, "a-two"), false)
            .status
            .success()
    );
    // A third session with no free recording to claim is dropped.
    assert!(
        sb.hook(&event("sessC", &proj_c, "c-one"), false)
            .status
            .success()
    );

    let alpha = sb.active_rec_id_by_name("alpha");
    let beta = sb.active_rec_id_by_name("beta");
    assert_ne!(alpha, beta);

    let span = |id: &str| {
        std::fs::read_to_string(sb.home().join(".galdr/spans").join(format!("{id}.jsonl"))).unwrap()
    };
    // Each span holds only its own session's steps.
    let a = span(&alpha);
    assert!(
        a.contains("a-one") && a.contains("a-two"),
        "alpha span: {a}"
    );
    assert!(
        !a.contains("b-one") && !a.contains("b-two") && !a.contains("c-one"),
        "alpha span leaked another session: {a}"
    );
    let b = span(&beta);
    assert!(b.contains("b-one") && b.contains("b-two"), "beta span: {b}");
    assert!(
        !b.contains("a-one") && !b.contains("c-one"),
        "beta span leaked another session: {b}"
    );
    assert_eq!(
        sb.span_lines(&alpha),
        2,
        "alpha recorded exactly its 2 steps"
    );
    assert_eq!(sb.span_lines(&beta), 2, "beta recorded exactly its 2 steps");

    // Stop alpha by name; beta keeps recording untouched.
    assert!(sb.run(&["rec", "stop", "alpha"]).status.success());
    let status = stdout(&sb.run(&["rec", "status"]));
    assert!(status.contains("1 active recording"), "{status}");
    assert!(status.contains("beta"), "{status}");
    // A further event from sessB still lands in beta.
    assert!(
        sb.hook(&event("sessB", &proj_b, "b-three"), false)
            .status
            .success()
    );
    assert_eq!(
        sb.span_lines(&beta),
        3,
        "beta kept recording after alpha stopped"
    );
    // alpha closed cleanly with its 2 steps.
    let list = stdout(&sb.run(&["list"]));
    assert!(
        list.contains("alpha"),
        "alpha should be a closed recording: {list}"
    );
}

#[test]
fn legacy_active_flag_is_migrated_without_losing_the_recording() {
    // An in-progress recording written under the old single ~/.galdr/active must
    // survive the upgrade: it is folded into active.d/ and keeps recording.
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.home().join(".galdr")).unwrap();
    // A fresh started_at: a stale recording would (correctly) stop taking
    // session-less events, and this test is about migration, not staleness.
    let started_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    std::fs::write(
        sb.home().join(".galdr/active"),
        format!(
            r#"{{"rec_id":"01LEGACY0000000000000000AA","name":"legacy-run","started_at":"{started_at}","origin_cwd":null,"bound_session":null}}"#
        ),
    )
    .unwrap();

    // Any command that touches the active set migrates it.
    let status = stdout(&sb.run(&["rec", "status"]));
    assert!(status.contains("legacy-run"), "{status}");
    assert!(status.contains("01LEGACY0000000000000000AA"), "{status}");
    // The legacy file is gone and the new per-recording flag exists.
    assert!(
        !sb.home().join(".galdr/active").exists(),
        "legacy flag must be removed"
    );
    assert!(
        sb.home()
            .join(".galdr/active.d/01LEGACY0000000000000000AA.json")
            .exists(),
        "recording must be folded into active.d/"
    );
    // It still records (sessionless event lands in it) and stops cleanly.
    assert!(sb.hook(BASH_STATUS, false).status.success());
    assert_eq!(sb.span_lines("01LEGACY0000000000000000AA"), 1);
    assert!(sb.run(&["rec", "stop", "legacy-run"]).status.success());
}

#[test]
fn rec_start_reaps_a_stale_recording() {
    // A recording whose session died without `rec stop` (started long ago, no
    // activity since) must not stay active forever: it would swallow session-less
    // events and scare every future session away from recording. The next
    // `rec start` closes it, with the reason on record.
    let sb = Sandbox::new();
    std::fs::create_dir_all(sb.home().join(".galdr/active.d")).unwrap();
    std::fs::write(
        sb.home()
            .join(".galdr/active.d/01ZOMBIE0000000000000000AA.json"),
        r#"{"rec_id":"01ZOMBIE0000000000000000AA","name":"zombie","started_at":"2026-01-01T00:00:00Z","origin_cwd":null,"bound_session":"dead-session"}"#,
    )
    .unwrap();

    let out = stdout(&sb.run(&["rec", "start", "fresh"]));
    assert!(out.contains("stale"), "{out}");
    // The zombie's flag is gone; its metadata records why it closed.
    assert!(
        !sb.home()
            .join(".galdr/active.d/01ZOMBIE0000000000000000AA.json")
            .exists(),
        "stale flag must be reaped"
    );
    let meta = std::fs::read_to_string(
        sb.home()
            .join(".galdr/recordings/01ZOMBIE0000000000000000AA.json"),
    )
    .unwrap();
    assert!(meta.contains("closed_reason"), "{meta}");
    assert!(meta.contains("stale"), "{meta}");
    // Only the new recording remains active.
    let status = stdout(&sb.run(&["rec", "status"]));
    assert!(status.contains("fresh"), "{status}");
    assert!(!status.contains("zombie"), "{status}");
}
