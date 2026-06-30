//! Human-observation recording lanes.
//!
//! `rec` records agent tool calls through harness hooks. `observe` is the second
//! sensor lane: human-demonstrated actions written as typed span events. The
//! first implementation is synthetic so the storage, catalog, distill, and
//! export paths can be exercised before a real browser sensor exists.

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use ulid::Ulid;

use crate::span::{
    Event, EventKind, HumanAction, HumanEvent, HumanSource, HumanTarget, HumanValue, TargetLocator,
};
use crate::{catalog, ipc, paths, record, span, style};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ObserveFixture {
    BrowserForm,
}

pub fn synthetic(name: String, fixture: ObserveFixture) -> Result<()> {
    paths::ensure_dirs()?;

    let rec_id = Ulid::new().to_string();
    let started_at = record::now_rfc3339();
    let events = match fixture {
        ObserveFixture::BrowserForm => browser_form_events(&started_at),
    };
    let recording = record::Recording {
        rec_id: rec_id.clone(),
        name,
        started_at,
        ended_at: record::now_rfc3339(),
        steps: events.len(),
        cwd: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
    };

    write_recording_files(&recording, &events)?;
    let _ = catalog::sync_closed_recording(&recording, &events);
    for event in &events {
        ipc::notify_best_effort(&ipc::Request::EventAppended {
            rec_id: recording.rec_id.clone(),
            event: Box::new(event.clone()),
        });
    }
    ipc::notify_best_effort(&ipc::Request::RecordingClosed {
        recording: recording.clone(),
    });

    println!(
        "{} observed \"{}\" — {} human steps",
        style::accent("◆"),
        recording.name,
        events.len()
    );
    println!("  rec_id: {}", recording.rec_id);
    println!(
        "  fixture: {}",
        match fixture {
            ObserveFixture::BrowserForm => "browser-form",
        }
    );
    println!(
        "  turn it into a skill:  galdr distill {}",
        recording.rec_id
    );
    Ok(())
}

fn write_recording_files(recording: &record::Recording, events: &[Event]) -> Result<()> {
    let span_path = paths::span_file(&recording.rec_id)?;
    let rec_path = paths::recording_file(&recording.rec_id)?;
    if span_path.exists() || rec_path.exists() {
        bail!("recording id collision: {}", recording.rec_id);
    }

    let span_tmp = span_path.with_extension("jsonl.tmp");
    let rec_tmp = rec_path.with_extension("json.tmp");
    let mut span_jsonl = String::new();
    for event in events {
        span_jsonl.push_str(&serde_json::to_string(event)?);
        span_jsonl.push('\n');
    }
    std::fs::write(&span_tmp, span_jsonl)
        .with_context(|| format!("could not write temporary span {}", span_tmp.display()))?;
    std::fs::rename(&span_tmp, &span_path)
        .with_context(|| format!("could not publish span {}", span_path.display()))?;
    let _ = span::fsync(&span_path);

    std::fs::write(&rec_tmp, serde_json::to_string_pretty(recording)?)
        .with_context(|| format!("could not write temporary recording {}", rec_tmp.display()))?;
    std::fs::rename(&rec_tmp, &rec_path)
        .with_context(|| format!("could not publish recording {}", rec_path.display()))?;
    Ok(())
}

fn browser_form_events(ts: &str) -> Vec<Event> {
    vec![
        human_event(
            ts,
            0,
            "human.browser.navigate",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.navigate"),
                target: None,
                value: None,
                verification_hint: None,
                frame_ref: None,
            },
        ),
        human_event(
            ts,
            1,
            "human.browser.input",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.input"),
                target: Some(label_target("Issue title")),
                value: Some(HumanValue::Redacted {
                    kind: "text".to_string(),
                    chars: Some(24),
                }),
                verification_hint: None,
                frame_ref: None,
            },
        ),
        human_event(
            ts,
            2,
            "human.browser.select",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.select"),
                target: Some(label_target("Priority")),
                value: Some(HumanValue::Literal {
                    value: "High".to_string(),
                }),
                verification_hint: None,
                frame_ref: None,
            },
        ),
        human_event(
            ts,
            3,
            "human.browser.click",
            HumanEvent {
                source: browser_source("https://example.test/issues/new", "New issue"),
                action: HumanAction::from("human.browser.click"),
                target: Some(role_target("button", "Create issue")),
                value: None,
                verification_hint: Some(
                    "Confirm the created issue page is open or a success message appears."
                        .to_string(),
                ),
                frame_ref: None,
            },
        ),
    ]
}

fn human_event(ts: &str, seq: u64, tool_name: &str, human: HumanEvent) -> Event {
    Event {
        ts: ts.to_string(),
        seq,
        tool_name: tool_name.to_string(),
        tool_input: serde_json::Value::Null,
        tool_response: serde_json::Value::Null,
        cwd: None,
        session_id: None,
        event_kind: EventKind::Human,
        human: Some(human),
    }
}

fn browser_source(url: &str, title: &str) -> HumanSource {
    HumanSource::Browser {
        url: Some(url.to_string()),
        title: Some(title.to_string()),
        tab_id: None,
    }
}

fn label_target(label: &str) -> HumanTarget {
    HumanTarget {
        primary: TargetLocator::Label {
            value: label.to_string(),
        },
        alternates: Vec::new(),
        role: None,
        name: None,
        text: None,
        label: Some(label.to_string()),
        placeholder: None,
        element_summary: None,
    }
}

fn role_target(role: &str, name: &str) -> HumanTarget {
    HumanTarget {
        primary: TargetLocator::Role {
            role: role.to_string(),
            name: Some(name.to_string()),
        },
        alternates: Vec::new(),
        role: Some(role.to_string()),
        name: Some(name.to_string()),
        text: None,
        label: None,
        placeholder: None,
        element_summary: None,
    }
}
