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
