//! End-to-end tests that drive the compiled `galdr` binary.
//!
//! Each test runs in its own temporary `HOME`, so `~/.galdr` and `~/.agents` are
//! isolated and the tests are hermetic and parallel-safe. The binary path comes
//! from `CARGO_BIN_EXE_galdr`, which cargo sets for integration tests.

#![allow(dead_code)]

pub(super) use std::io::{Read, Write};
pub(super) use std::net::TcpStream;
pub(super) use std::path::{Path, PathBuf};
pub(super) use std::process::{Command, Output, Stdio};

pub(super) fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_galdr")
}

/// An isolated `HOME` with a `galdr` command builder.
pub(super) struct Sandbox {
    home: tempfile::TempDir,
}

impl Sandbox {
    pub(super) fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }

    pub(super) fn home(&self) -> &Path {
        self.home.path()
    }

    pub(super) fn cmd(&self) -> Command {
        let mut command = Command::new(bin());
        command.env("HOME", self.home.path());
        command
    }

    pub(super) fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    /// Feeds a PostToolUse event to the sensor on stdin.
    pub(super) fn hook(&self, json: &str, fail: bool) -> Output {
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

    pub(super) fn span_lines(&self, rec_id: &str) -> usize {
        let path = self
            .home()
            .join(".galdr/spans")
            .join(format!("{rec_id}.jsonl"));
        std::fs::read_to_string(path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }

    /// The rec_id of the sole in-progress recording (read from its `active.d/` flag,
    /// since `recordings/` is only written on stop). Panics if not exactly one is
    /// active — the callers all record a single recording.
    pub(super) fn active_rec_id(&self) -> String {
        let dir = self.home().join(".galdr/active.d");
        let mut ids: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .map(|e| {
                let raw = std::fs::read_to_string(e.path()).unwrap();
                let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
                value["rec_id"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(ids.len(), 1, "expected exactly one active recording");
        ids.pop().unwrap()
    }

    /// The rec_id of the active recording with the given name (for concurrency tests
    /// where several recordings are active at once).
    pub(super) fn active_rec_id_by_name(&self, name: &str) -> String {
        let dir = self.home().join(".galdr/active.d");
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            if entry.path().extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(entry.path()).unwrap();
            let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
            if value["name"].as_str() == Some(name) {
                return value["rec_id"].as_str().unwrap().to_string();
            }
        }
        panic!("no active recording named {name}");
    }

    pub(super) fn recording_ids(&self) -> Vec<String> {
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
    pub(super) fn record(&self, name: &str, events: &[&str]) -> String {
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

    pub(super) fn skill_md(&self, skill_name: &str) -> String {
        let path = self
            .home()
            .join(".agents/skills")
            .join(skill_name)
            .join("SKILL.md");
        std::fs::read_to_string(path).unwrap()
    }
}

/// Writes a crates.io sparse-index fixture (one NDJSON line per entry) and returns
/// its path, for driving `galdr upgrade`/`doctor` hermetically via `GALDR_INDEX_FILE`
/// — no network, deterministic across machines.
pub(super) fn write_index_fixture(sb: &Sandbox, entries: &[(&str, bool)]) -> PathBuf {
    let path = sb.home().join("index.json");
    let mut body = String::new();
    for (vers, yanked) in entries {
        body.push_str(&format!(
            r#"{{"name":"galdr","vers":"{vers}","deps":[],"cksum":"x","features":{{}},"yanked":{yanked}}}"#
        ));
        body.push('\n');
    }
    std::fs::write(&path, body).unwrap();
    path
}

/// Writes an always-succeeding `launchctl` stand-in that appends its arguments to a
/// log file, and returns `(script_path, log_path)`. Point `GALDR_LAUNCHCTL` at the
/// script so `galdr daemon install`/`status`/`uninstall` exercise the real command
/// wiring (arg construction, exit-code handling) without touching the live launchd.
#[cfg(target_os = "macos")]
pub(super) fn fake_launchctl(sb: &Sandbox) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = sb.home().join("launchctl.log");
    let script = sb.home().join("fake-launchctl.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexit 0\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, log)
}

pub(super) fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(super) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(super) const BASH_STATUS: &str =
    r#"{"tool_name":"Bash","tool_input":{"command":"git status"},"tool_response":{}}"#;

pub(super) fn write_human_recording(
    sb: &Sandbox,
    rec_id: &str,
    name: &str,
    value: serde_json::Value,
) {
    let root = sb.home().join(".galdr");
    std::fs::create_dir_all(root.join("spans")).unwrap();
    std::fs::create_dir_all(root.join("recordings")).unwrap();
    let event = serde_json::json!({
        "ts": "2026-06-30T00:00:00Z",
        "seq": 0,
        "tool_name": "human.browser.input",
        "tool_input": {},
        "tool_response": {},
        "event_kind": "human",
        "human": {
            "source": {
                "kind": "browser",
                "url": "https://example.test/issues/new",
                "title": "New issue"
            },
            "action": "human.browser.input",
            "target": {
                "primary": {
                    "kind": "label",
                    "value": "Issue title"
                },
                "label": "Issue title"
            },
            "value": value,
            "verification_hint": "Confirm the issue was saved."
        }
    });
    std::fs::write(
        root.join("spans").join(format!("{rec_id}.jsonl")),
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .unwrap();
    let recording = serde_json::json!({
        "rec_id": rec_id,
        "name": name,
        "started_at": "2026-06-30T00:00:00Z",
        "ended_at": "2026-06-30T00:01:00Z",
        "steps": 1,
        "cwd": null
    });
    std::fs::write(
        root.join("recordings").join(format!("{rec_id}.json")),
        serde_json::to_string_pretty(&recording).unwrap(),
    )
    .unwrap();
}

pub(super) fn active_browser_port(sb: &Sandbox) -> u16 {
    let raw =
        std::fs::read_to_string(sb.home().join(".galdr/observe/browser-active.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["port"].as_u64().unwrap() as u16
}

pub(super) fn post_browser_event(port: u16, event: serde_json::Value) {
    let body = serde_json::to_string(&event).unwrap();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let request = format!(
        "POST /event HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 204"),
        "unexpected response: {response}"
    );
}
