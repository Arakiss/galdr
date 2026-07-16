#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::catalog::{RecordingDetail, RecordingRow, SkillRow, StepRow};
    use crate::span::Event;
    use crate::tui::app::App;
    use crate::tui::data::Catalog;

    struct MockCatalog {
        recordings: Vec<RecordingRow>,
        skills: Vec<SkillRow>,
        detail: Option<RecordingDetail>,
        raw: Vec<Event>,
    }

    impl Catalog for MockCatalog {
        fn recordings(&self) -> Vec<RecordingRow> {
            self.recordings.clone()
        }
        fn detail(&self, _rec_id: &str) -> Option<RecordingDetail> {
            self.detail.clone()
        }
        fn raw_events(&self, _rec_id: &str) -> Vec<Event> {
            self.raw.clone()
        }
        fn skills(&self) -> Vec<SkillRow> {
            self.skills.clone()
        }
    }

    fn rec_row(id: &str, name: &str, distilled: bool) -> RecordingRow {
        RecordingRow {
            rec_id: id.into(),
            name: name.into(),
            started_at: "2026-06-19T10:00:00Z".into(),
            ended_at: Some("2026-06-19T10:05:00Z".into()),
            steps: 1,
            cwd: Some("/proj/demo".into()),
            distilled,
        }
    }

    fn fixture() -> MockCatalog {
        let recording = rec_row("01AAA", "tui demo", true);
        let detail = RecordingDetail {
            recording: recording.clone(),
            steps: vec![StepRow {
                seq: 0,
                tool_name: "Bash".into(),
                event_kind: "tool_call".into(),
                ts: "2026-06-19T10:00:01Z".into(),
                summary: "git status".into(),
                judgments: Vec::new(),
            }],
        };
        MockCatalog {
            recordings: vec![recording],
            skills: vec![
                SkillRow {
                    skill_name: "galdr-tui-demo".into(),
                    rec_id: Some("01AAA".into()),
                    skill_path: "/x/SKILL.md".into(),
                    installed_at: None,
                    status: crate::catalog::STATUS_FINAL.into(),
                    readiness_score: 100,
                    readiness_delta: 0,
                    readiness_notes: "ready".into(),
                    orphan: false,
                    origin: crate::catalog::ORIGIN_GALDR.into(),
                },
                SkillRow {
                    skill_name: "bun".into(),
                    rec_id: None,
                    skill_path: "/x/bun/SKILL.md".into(),
                    installed_at: None,
                    status: crate::catalog::STATUS_UNKNOWN.into(),
                    readiness_score: 60,
                    readiness_delta: 0,
                    readiness_notes: "external".into(),
                    orphan: true,
                    origin: crate::catalog::ORIGIN_EXTERNAL.into(),
                },
            ],
            detail: Some(detail),
            raw: vec![Event {
                ts: "2026-06-19T10:00:01Z".into(),
                seq: 0,
                tool_name: "Bash".into(),
                tool_input: serde_json::json!({ "command": "git status" }),
                tool_response: serde_json::json!({ "exit_code": 0 }),
                cwd: Some("/proj/demo".into()),
                session_id: None,
                event_kind: crate::span::EventKind::ToolCall,
                human: None,
            }],
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn render_text(app: &mut App<MockCatalog>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn landing_shows_the_overview() {
        // The TUI opens on the Overview tab — a dashboard that orients you, not a raw
        // list. The tab bar names every tab; switching to Recordings shows the steps.
        let mut app = App::new(fixture());
        let s = render_text(&mut app);
        assert!(s.contains("Overview"));
        assert!(s.contains("Recent recordings"));
        assert!(s.contains("tui demo")); // the recording, listed in the overview
        assert!(s.contains("Your skills"));
        assert!(s.contains("Recordings") && s.contains("Skills")); // the tab bar
        // Tab 2 (Recordings) opens the steps inspector.
        app.on_key(key(KeyCode::Char('2')));
        let rec = render_text(&mut app);
        assert!(rec.contains("Steps"));
        assert!(rec.contains("git status"));
    }

    #[test]
    fn recording_detail_hides_setup_and_noise_steps() {
        // A `galdr rec` control command is recorded but is capture noise, not a task
        // step. The inspector must show only the meaningful steps and report how many it
        // hid, so a human reads the task — not the scaffolding of its capture.
        let recording = rec_row("01BBB", "noisy", true);
        let detail = RecordingDetail {
            recording: recording.clone(),
            steps: vec![
                StepRow {
                    seq: 0,
                    tool_name: "Bash".into(),
                    event_kind: "tool_call".into(),
                    ts: "t".into(),
                    summary: "cargo build".into(),
                    judgments: Vec::new(),
                },
                StepRow {
                    seq: 1,
                    tool_name: "Bash".into(),
                    event_kind: "tool_call".into(),
                    ts: "t".into(),
                    summary: "galdr rec status".into(),
                    judgments: Vec::new(),
                },
            ],
        };
        let raw = vec![
            Event {
                ts: "t".into(),
                seq: 0,
                tool_name: "Bash".into(),
                tool_input: serde_json::json!({ "command": "cargo build" }),
                tool_response: serde_json::json!({}),
                cwd: None,
                session_id: None,
                event_kind: crate::span::EventKind::ToolCall,
                human: None,
            },
            Event {
                ts: "t".into(),
                seq: 1,
                tool_name: "Bash".into(),
                tool_input: serde_json::json!({ "command": "galdr rec status" }),
                tool_response: serde_json::json!({}),
                cwd: None,
                session_id: None,
                event_kind: crate::span::EventKind::ToolCall,
                human: None,
            },
        ];
        let mock = MockCatalog {
            recordings: vec![recording],
            skills: vec![],
            detail: Some(detail),
            raw,
        };
        // The Recordings tab (2) selects the first recording and projects its detail.
        let mut app = App::new(mock);
        app.on_key(key(KeyCode::Char('2')));
        assert_eq!(app.hidden_steps, 1, "the galdr control command is hidden");
        let steps = &app.detail.as_ref().unwrap().steps;
        assert_eq!(steps.len(), 1, "only the meaningful step remains");
        assert_eq!(steps[0].summary, "cargo build");
    }

    #[test]
    fn panel_keys_move_focus_and_the_preview_follows() {
        let mut app = App::new(fixture());
        // Focus Skills (tab 3); the body is the skills list + the skill's SKILL.md.
        app.on_key(key(KeyCode::Char('3')));
        let skills = render_text(&mut app);
        assert!(skills.contains("galdr-tui-demo"));
        assert!(skills.contains("extern")); // the bun skill is marked external
        assert!(skills.contains("Skill ·")); // the preview pane is now a skill preview

        // Focus Harnesses (tab 4); the body switches to the harness detail.
        app.on_key(key(KeyCode::Char('4')));
        let harn = render_text(&mut app);
        assert!(harn.contains("Claude Code"));
        assert!(harn.contains("Harness ·"));
    }

    #[test]
    fn enter_focuses_the_preview_and_opens_a_step_raw() {
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('2'))); // Recordings tab
        let insp = render_text(&mut app);
        assert!(insp.contains("Inspector"));
        assert!(insp.contains("Steps"));
        assert!(insp.contains("git status"));

        app.on_key(key(KeyCode::Enter)); // focus the preview (select a step)
        app.on_key(key(KeyCode::Enter)); // raw overlay for that step
        let raw = render_text(&mut app);
        assert!(raw.contains("sensitive"));
        assert!(raw.contains("tool_input"));
    }

    #[test]
    fn filter_narrows_the_skills_list() {
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('3'))); // Skills
        app.on_key(key(KeyCode::Char('/')));
        for c in "tui".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        app.on_key(key(KeyCode::Enter));
        let s = render_text(&mut app);
        assert!(s.contains("galdr-tui-demo"));
        assert!(!s.contains(" bun "));
    }

    #[test]
    fn validate_action_targets_the_selected_skill() {
        // Pressing `v` on the Skills panel validates the selected skill. The fixture's
        // skill_path doesn't exist, so it reports it can't read it — proving the action
        // is wired to the selection, with no filesystem writes (validate is read-only).
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('3'))); // Skills
        app.on_key(key(KeyCode::Char('v'))); // validate
        assert!(app.status.contains("galdr-tui-demo"), "{}", app.status);
    }

    #[test]
    fn skill_preview_scrolls_when_focused() {
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('3'))); // Skills
        app.preview_md = "a\nb\nc\nd\ne".to_string(); // simulate a loaded SKILL.md
        app.on_key(key(KeyCode::Enter)); // focus the preview to scroll
        assert!(app.preview_focus);
        app.on_key(key(KeyCode::Char('j')));
        app.on_key(key(KeyCode::Char('j')));
        assert_eq!(app.preview_scroll, 2);
        app.on_key(key(KeyCode::Char('g')));
        assert_eq!(app.preview_scroll, 0);
        app.on_key(key(KeyCode::Esc));
        assert!(!app.preview_focus);
    }

    #[test]
    fn quit_sets_should_quit() {
        let mut app = App::new(fixture());
        assert!(!app.should_quit);
        app.on_key(key(KeyCode::Char('q')));
        assert!(app.should_quit);
    }

    #[test]
    fn help_overlay_renders_and_closes() {
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('?')));
        let help = render_text(&mut app);
        assert!(help.contains("keybindings"));
        app.on_key(key(KeyCode::Esc));
        assert!(app.overlay.is_none());
    }
}
