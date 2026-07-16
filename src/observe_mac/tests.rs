#[cfg(test)]
mod tests {
    use super::*;

    fn sample_wire(action: &str) -> MacWireEvent {
        MacWireEvent {
            ts: "2026-07-02T00:00:00Z".into(),
            action: action.into(),
            x: Some(120.0),
            y: Some(48.0),
            button: None,
            scroll_delta: None,
            role: None,
            name: None,
            window: None,
            app: None,
        }
    }

    #[test]
    fn wire_click_maps_to_human_click() {
        let mut wire = sample_wire("human.mac.click");
        wire.button = Some(0);
        let event = mac_wire_to_event(0, wire);
        assert_eq!(event.event_kind, EventKind::Human);
        assert_eq!(event.tool_name, "human.mac.click");
        let human = event.human.expect("human payload");
        assert_eq!(human.action.as_str(), "human.mac.click");
        assert!(matches!(human.source, HumanSource::MacApp { .. }));
        let target = human.target.expect("coordinate summary target");
        assert_eq!(target.element_summary.as_deref(), Some("screen (120, 48)"));
        // A click carries no value: nothing typed, nothing to redact.
        assert!(human.value.is_none());
    }

    #[test]
    fn wire_click_with_ax_builds_semantic_target() {
        let mut wire = sample_wire("human.mac.click");
        wire.role = Some("AXButton".into());
        wire.name = Some("Enviar".into());
        wire.window = Some("Gastos".into());
        wire.app = Some("Contsimple".into());
        let event = mac_wire_to_event(0, wire);
        let human = event.human.expect("human payload");
        // The app and window ride on the source; the role/name become the locator.
        match human.source {
            HumanSource::MacApp { app, window_title } => {
                assert_eq!(app.as_deref(), Some("Contsimple"));
                assert_eq!(window_title.as_deref(), Some("Gastos"));
            }
            other => panic!("expected MacApp source, got {other:?}"),
        }
        let target = human.target.expect("semantic target");
        assert_eq!(target.role.as_deref(), Some("AXButton"));
        assert_eq!(target.name.as_deref(), Some("Enviar"));
        match target.primary {
            TargetLocator::Role { role, name } => {
                assert_eq!(role, "AXButton");
                assert_eq!(name.as_deref(), Some("Enviar"));
            }
            other => panic!("expected a role locator, got {other:?}"),
        }
    }

    #[test]
    fn wire_key_omits_content_and_has_no_bogus_target() {
        // The real sensor stamps the mouse position on key events too; sample_wire keeps
        // x/y set to mirror that. A keystroke must still record no content and no target.
        let wire = sample_wire("human.mac.key");
        let event = mac_wire_to_event(3, wire);
        assert_eq!(event.seq, 3);
        let human = event.human.expect("human payload");
        // The keystroke is recorded as an occurrence; its content is omitted by policy.
        match human.value {
            Some(HumanValue::Omitted { .. }) => {}
            other => panic!("expected omitted key value, got {other:?}"),
        }
        // No AX role resolved for a key, so the unrelated mouse coordinate must NOT become
        // a spurious target.
        assert!(
            human.target.is_none(),
            "a keystroke must not carry a coordinate target"
        );
    }

    #[test]
    fn wire_event_ndjson_roundtrips() {
        let mut wire = sample_wire("human.mac.scroll");
        wire.scroll_delta = Some(-3);
        wire.role = Some("AXScrollArea".into());
        let line = serde_json::to_string(&wire).unwrap();
        let back: MacWireEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(wire, back);
    }
}
