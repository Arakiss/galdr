mod common;

use common::*;

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
    // Keep the update check hermetic: read the "index" from a local fixture pinned to
    // this build's version, so doctor never touches the network and the update line is
    // deterministic ("up to date").
    let index = write_index_fixture(&sb, &[(env!("CARGO_PKG_VERSION"), false)]);
    let doctor = sb
        .cmd()
        .env("GALDR_INDEX_FILE", &index)
        .arg("doctor")
        .output()
        .unwrap();
    assert!(
        doctor.status.success(),
        "{}\n{}",
        stdout(&doctor),
        String::from_utf8_lossy(&doctor.stderr)
    );
    let said = stdout(&doctor);
    assert!(said.contains("doctor: ok"));
    assert!(
        said.contains(&format!("up to date (v{})", env!("CARGO_PKG_VERSION"))),
        "doctor should surface the update check: {said}"
    );
}

#[test]
fn upgrade_check_reports_up_to_date() {
    let sb = Sandbox::new();
    let index = write_index_fixture(
        &sb,
        &[("0.14.0", false), (env!("CARGO_PKG_VERSION"), false)],
    );
    let out = sb
        .cmd()
        .env("GALDR_INDEX_FILE", &index)
        .args(["upgrade", "--check"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("is up to date"), "{}", stdout(&out));
}

#[test]
fn upgrade_check_flags_a_newer_version_with_exit_10() {
    // A published version greater than this build: --check names it, points at the
    // command, and exits 10 — a distinct signal a script can branch on.
    let sb = Sandbox::new();
    let index = write_index_fixture(&sb, &[(env!("CARGO_PKG_VERSION"), false), ("9.9.9", false)]);
    let out = sb
        .cmd()
        .env("GALDR_INDEX_FILE", &index)
        .args(["upgrade", "--check"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(10), "{}", stderr(&out));
    let said = stdout(&out);
    assert!(said.contains("9.9.9"), "{said}");
    assert!(said.contains("galdr upgrade"), "{said}");
}

#[test]
fn upgrade_check_ignores_a_yanked_newer_version() {
    // The only newer version is yanked, so the live max equals this build: up to date.
    let sb = Sandbox::new();
    let index = write_index_fixture(&sb, &[(env!("CARGO_PKG_VERSION"), false), ("9.9.9", true)]);
    let out = sb
        .cmd()
        .env("GALDR_INDEX_FILE", &index)
        .args(["upgrade", "--check"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("is up to date"), "{}", stdout(&out));
}

#[test]
fn upgrade_check_reports_local_ahead() {
    // The published max is behind this build — the normal state when running from a
    // clone whose version bump has not shipped yet. Not an update, exit 0.
    let sb = Sandbox::new();
    let index = write_index_fixture(&sb, &[("0.0.1", false)]);
    let out = sb
        .cmd()
        .env("GALDR_INDEX_FILE", &index)
        .args(["upgrade", "--check"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let said = stdout(&out);
    assert!(said.contains("ahead of crates.io"), "{said}");
    assert!(said.contains("v0.0.1"), "{said}");
}

#[test]
fn upgrade_check_is_soft_when_offline() {
    // A missing index fixture stands in for "no network": a note, exit 0 — never an
    // error. galdr's local-first contract makes offline a note, not a failure.
    let sb = Sandbox::new();
    let missing = sb.home().join("no-such-index.json");
    let out = sb
        .cmd()
        .env("GALDR_INDEX_FILE", &missing)
        .args(["upgrade", "--check"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("offline"), "{}", stdout(&out));
}

#[test]
fn upgrade_rejects_an_invalid_from() {
    // A malformed `--from` is a usage error (exit 1) and never reaches the network.
    let sb = Sandbox::new();
    let out = sb
        .cmd()
        .env("GALDR_INDEX_FILE", sb.home().join("unused.json"))
        .args(["upgrade", "--check", "--from", "bogus"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--from"), "{}", stderr(&out));
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
    let status = stdout(&sb.run(&["daemon", "status"]));
    assert!(status.contains("daemon running"));
    // The daemon reports its version over the socket (so a stale daemon left running
    // from an older binary is detectable); it matches this build's version.
    assert!(
        status.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))),
        "daemon status should print the version: {status}"
    );

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

#[cfg(target_os = "macos")]
#[test]
fn daemon_install_writes_loads_and_uninstalls_the_launchagent() {
    let sb = Sandbox::new();
    let (launchctl, log) = fake_launchctl(&sb);

    let install = sb
        .cmd()
        .env("GALDR_LAUNCHCTL", &launchctl)
        .args(["daemon", "install"])
        .output()
        .unwrap();
    assert!(install.status.success(), "{}", stderr(&install));

    // The plist landed under this HOME's LaunchAgents, wired to run this binary as a
    // daemon and to keep it alive.
    let plist = sb
        .home()
        .join("Library/LaunchAgents/dev.galdr.daemon.plist");
    assert!(plist.exists(), "the LaunchAgent plist must be written");
    let body = std::fs::read_to_string(&plist).unwrap();
    assert!(body.contains("<string>dev.galdr.daemon</string>"), "{body}");
    assert!(
        body.contains(bin()),
        "plist must point at this binary: {body}"
    );
    assert!(body.contains("<string>daemon</string>"), "{body}");
    assert!(body.contains("<key>KeepAlive</key>"), "{body}");
    assert!(body.contains("<key>RunAtLoad</key>"), "{body}");
    // The log directory was created, and a first install bootstraps the job.
    assert!(sb.home().join(".galdr/logs").is_dir(), "log dir must exist");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("bootstrap"), "expected bootstrap: {logged}");

    // status now reports it as managed (plist present + the probe exits 0).
    let status = sb
        .cmd()
        .env("GALDR_LAUNCHCTL", &launchctl)
        .args(["daemon", "status"])
        .output()
        .unwrap();
    assert!(
        stdout(&status).contains("launchd: managed"),
        "{}",
        stdout(&status)
    );

    // uninstall unloads the job and removes the plist, but leaves logs/state.
    let uninstall = sb
        .cmd()
        .env("GALDR_LAUNCHCTL", &launchctl)
        .args(["daemon", "uninstall"])
        .output()
        .unwrap();
    assert!(uninstall.status.success(), "{}", stderr(&uninstall));
    assert!(!plist.exists(), "the plist must be removed on uninstall");
    assert!(
        sb.home().join(".galdr/logs").is_dir(),
        "uninstall must not delete logs/state"
    );
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("bootout"), "expected bootout: {logged}");
}

#[cfg(target_os = "macos")]
#[test]
fn daemon_status_reports_unmanaged_without_a_launchagent() {
    // No plist installed → unmanaged, and (by design) launchctl is never invoked, so
    // the check stays hermetic without any stand-in.
    let sb = Sandbox::new();
    let status = sb.run(&["daemon", "status"]);
    assert!(status.status.success());
    assert!(
        stdout(&status).contains("launchd: unmanaged"),
        "{}",
        stdout(&status)
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn daemon_install_is_macos_only() {
    // launchd is macOS-only: the subcommand must fail with a clear message elsewhere.
    let sb = Sandbox::new();
    let out = sb.run(&["daemon", "install"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("macOS-only"), "{}", stderr(&out));
}
