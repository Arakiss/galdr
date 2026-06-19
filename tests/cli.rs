//! End-to-end tests that drive the compiled `galdr` binary.
//!
//! Each test runs in its own temporary `HOME`, so `~/.galdr` and `~/.agents` are
//! isolated and the tests are hermetic and parallel-safe. The binary path comes
//! from `CARGO_BIN_EXE_galdr`, which cargo sets for integration tests.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_galdr")
}

/// An isolated `HOME` with a `galdr` command builder.
struct Sandbox {
    home: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> &Path {
        self.home.path()
    }

    fn cmd(&self) -> Command {
        let mut command = Command::new(bin());
        command.env("HOME", self.home.path());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    /// Feeds a PostToolUse event to the sensor on stdin.
    fn hook(&self, json: &str, fail: bool) -> Output {
        let mut command = self.cmd();
        command.arg("hook");
        if fail {
            command.env("GALDR_HOOK_FAIL", "1");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(json.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn span_lines(&self, rec_id: &str) -> usize {
        let path = self
            .home()
            .join(".galdr/spans")
            .join(format!("{rec_id}.jsonl"));
        std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }

    /// The rec_id of the in-progress recording (read from the `active` flag,
    /// since `recordings/` is only written on stop).
    fn active_rec_id(&self) -> String {
        let raw = std::fs::read_to_string(self.home().join(".galdr/active")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["rec_id"].as_str().unwrap().to_string()
    }

    fn recording_ids(&self) -> Vec<String> {
        let dir = self.home().join(".galdr/recordings");
        let mut ids: Vec<String> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let path = entry.path();
                        if path.extension()?.to_str()? == "json" {
                            Some(path.file_stem()?.to_str()?.to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// Records a sequence of events under `name` and returns the rec_id.
    fn record(&self, name: &str, events: &[&str]) -> String {
        let before = self.recording_ids();
        assert!(self.run(&["rec", "start", name]).status.success());
        for event in events {
            assert!(self.hook(event, false).status.success());
        }
        assert!(self.run(&["rec", "stop"]).status.success());
        self.recording_ids()
            .into_iter()
            .find(|id| !before.contains(id))
            .expect("a new recording id")
    }

    fn skill_md(&self, skill_name: &str) -> String {
        let path = self
            .home()
            .join(".agents/skills")
            .join(skill_name)
            .join("SKILL.md");
        std::fs::read_to_string(path).unwrap()
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const BASH_STATUS: &str =
    r#"{"tool_name":"Bash","tool_input":{"command":"git status"},"tool_response":{}}"#;

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
            "---\nname: galdr-demo\ndescription: \"does a thing\"\n---\n\n## Provenance\n- rec_id: `{id}`\n\n## Goal\nx\n## Procedure\ny\n"
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
fn distill_auto_falls_back_to_the_draft_without_an_engine() {
    let sb = Sandbox::new();
    let id = sb.record("auto demo", &[BASH_STATUS]);

    // No MLX server and no Python mlx_lm: --auto must fall back and exit 0.
    let auto = sb.run(&["distill", &id, "--auto"]);
    assert!(
        auto.status.success(),
        "--auto must exit 0 even with no engine"
    );
    let draft = sb.skill_md("galdr-auto-demo");
    assert!(draft.contains("galdr-auto-demo"));
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

/// Optional daemon round-trip. Kept robust (generous polling, guaranteed
/// teardown) but isolated to its own temp HOME so it never disturbs the others.
#[test]
fn daemon_indexes_and_answers_queries() {
    let sb = Sandbox::new();
    let pidfile = sb.home().join(".galdr/galdrd.pid");
    let socket = sb.home().join(".galdr/galdrd.sock");

    assert!(sb.run(&["daemon", "--detach"]).status.success());

    // A guard that kills the daemon on the way out, even if an assert fails.
    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(pid) = std::fs::read_to_string(&self.0)
                && let Ok(pid) = pid.trim().parse::<i32>()
            {
                let _ = Command::new("kill").arg(pid.to_string()).status();
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

    sb.record("daemon demo", &[BASH_STATUS]);
    // Give the close notification a moment to be indexed.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let list = sb.run(&["list"]);
    assert!(list.status.success());
    assert!(
        stdout(&list).contains("daemon demo"),
        "the daemon-backed catalog should list the recording"
    );
}
