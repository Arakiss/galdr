//! Rendering for the three screens and the modal overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::{App, Overlay, Screen};
use super::data::Catalog;
use super::theme;

pub fn render<C: Catalog>(frame: &mut Frame, app: &mut App<C>) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    render_title(frame, chunks[0], app);
    match app.screen {
        Screen::Recordings => render_recordings(frame, chunks[1], app),
        Screen::Detail => render_detail(frame, chunks[1], app),
        Screen::Audit => render_audit(frame, chunks[1], app),
    }
    render_status(frame, chunks[2], app);

    if let Some(overlay) = app.overlay.as_ref() {
        render_overlay(frame, area, app, overlay);
    }
}

fn block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {title} "), theme::title()))
}

fn render_title<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let screen = match app.screen {
        Screen::Recordings => "recordings",
        Screen::Detail => "inspector",
        Screen::Audit => "audit",
    };
    let line = Line::from(vec![
        Span::styled("galdr", theme::title()),
        Span::styled("  record & replay for agent skills", theme::dim()),
        Span::raw("   ·   "),
        Span::styled(screen, theme::ok()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_status<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let hints = match app.screen {
        Screen::Recordings => {
            "↑↓/jk move · enter inspect · d distill · a audit · r replay · o span · ? help · q quit"
        }
        Screen::Detail => "↑↓/jk step · enter raw · o span · esc back · ? help · q quit",
        Screen::Audit => "↑↓/jk move · esc back · ? help · q quit",
    };
    let line = if app.status.is_empty() {
        Line::styled(hints, theme::dim())
    } else {
        Line::from(vec![
            Span::styled(format!("{}  ", app.status), theme::ok()),
            Span::styled(hints, theme::dim()),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_recordings<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    if app.recordings.is_empty() {
        let empty = Paragraph::new("No recordings yet. Record one with `galdr rec start <name>`.")
            .style(theme::dim())
            .block(block("Recordings"));
        frame.render_widget(empty, area);
        return;
    }

    let header = Row::new(["", "rec_id", "name", "steps", "recorded"]).style(theme::dim());
    let rows: Vec<Row> = app
        .recordings
        .iter()
        .map(|r| {
            let mark = if r.distilled {
                Span::styled("✓", theme::ok())
            } else {
                Span::raw(" ")
            };
            Row::new(vec![
                Cell::from(mark),
                Cell::from(r.rec_id.clone()),
                Cell::from(r.name.clone()),
                Cell::from(r.steps.to_string()),
                Cell::from(short_ts(&r.started_at)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Length(28),
        Constraint::Min(14),
        Constraint::Length(6),
        Constraint::Length(20),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌ ")
        .block(block("Recordings"));
    frame.render_stateful_widget(table, area, &mut app.rec_state);
}

fn render_detail<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    let Some(detail) = app.detail.as_ref() else {
        frame.render_widget(
            Paragraph::new("(no recording)").block(block("Inspector")),
            area,
        );
        return;
    };
    let rec = &detail.recording;

    let chunks = Layout::vertical([Constraint::Length(4), Constraint::Min(1)]).split(area);

    let mut meta = vec![
        Line::from(vec![
            Span::styled(rec.rec_id.clone(), theme::title()),
            Span::raw("  "),
            Span::styled(rec.name.clone(), theme::ok()),
        ]),
        Line::styled(
            format!(
                "recorded {} → {}",
                short_ts(&rec.started_at),
                rec.ended_at
                    .as_deref()
                    .map(short_ts)
                    .unwrap_or_else(|| "(open)".into())
            ),
            theme::dim(),
        ),
    ];
    meta.push(Line::styled(
        format!(
            "distilled: {}   ·   cwd: {}",
            if rec.distilled { "yes" } else { "no" },
            rec.cwd.as_deref().unwrap_or("-")
        ),
        theme::dim(),
    ));
    frame.render_widget(Paragraph::new(meta).block(block("Inspector")), chunks[0]);

    let header = Row::new(["#", "tool", "summary"]).style(theme::dim());
    let rows: Vec<Row> = detail
        .steps
        .iter()
        .map(|s| {
            Row::new(vec![
                Cell::from((s.seq + 1).to_string()),
                Cell::from(s.tool_name.clone()),
                Cell::from(s.summary.clone()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(4),
        Constraint::Length(12),
        Constraint::Min(20),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌ ")
        .block(block("Steps"));
    frame.render_stateful_widget(table, chunks[1], &mut app.detail_state);
}

fn render_audit<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    if app.skills.is_empty() {
        let empty =
            Paragraph::new("No skills distilled yet. Press `d` on a recording to draft one.")
                .style(theme::dim())
                .block(block("Audit · provenance"));
        frame.render_widget(empty, area);
        return;
    }

    let header = Row::new(["skill", "← recording", "status"]).style(theme::dim());
    let rows: Vec<Row> = app
        .skills
        .iter()
        .map(|s| {
            let status = if s.orphan {
                Span::styled("orphan", theme::warn())
            } else {
                Span::styled("linked", theme::ok())
            };
            Row::new(vec![
                Cell::from(s.skill_name.clone()),
                Cell::from(s.rec_id.clone().unwrap_or_else(|| "(none)".into())),
                Cell::from(status),
            ])
        })
        .collect();
    let widths = [
        Constraint::Min(20),
        Constraint::Length(28),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌ ")
        .block(block("Audit · provenance"));
    frame.render_stateful_widget(table, area, &mut app.audit_state);
}

fn render_overlay<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>, overlay: &Overlay) {
    let (title, body, warn) = match overlay {
        Overlay::Raw(i) => (
            "raw — may contain sensitive data".to_string(),
            raw_body(app, *i),
            true,
        ),
        Overlay::Replay => ("replay".to_string(), replay_body(app), false),
        Overlay::Help => ("keybindings".to_string(), help_body(), false),
    };
    let rect = centered(area, 80, 70);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {title} "),
            if warn { theme::warn() } else { theme::title() },
        ))
        .title_bottom(Span::styled(" esc to close ", theme::dim()));
    let para = Paragraph::new(body).block(block).wrap(Wrap { trim: false });
    frame.render_widget(para, rect);
}

fn raw_body<C: Catalog>(app: &App<C>, i: usize) -> Text<'static> {
    let Some(event) = app.raw.get(i) else {
        return Text::from("(raw event unavailable)");
    };
    let pretty =
        |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    let mut lines = vec![
        Line::styled(
            "This is the raw recorded payload, straight from the span. Treat it as sensitive.",
            theme::warn(),
        ),
        Line::raw(""),
        Line::styled(
            format!("step {} · {}", event.seq + 1, event.tool_name),
            theme::ok(),
        ),
        Line::raw(""),
        Line::styled("tool_input", theme::dim()),
    ];
    for l in pretty(&event.tool_input).lines() {
        lines.push(Line::raw(l.to_string()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("tool_response", theme::dim()));
    for l in pretty(&event.tool_response).lines() {
        lines.push(Line::raw(l.to_string()));
    }
    Text::from(lines)
}

fn replay_body<C: Catalog>(app: &App<C>) -> Text<'static> {
    let distilled = app
        .selected_recording()
        .map(|r| r.distilled)
        .unwrap_or(false);
    let mut lines = vec![
        Line::styled("galdr does not re-execute tool calls.", theme::title()),
        Line::raw(""),
        Line::raw(
            "A GUI macro replays pixels and clicks; that breaks the moment anything moves. \
             galdr is not that. \"Replay\" here means: the recording is distilled into a skill, \
             and the agent reproduces the task by reading that skill and applying judgment — \
             adapting paths, names, and order to the situation in front of it.",
        ),
        Line::raw(""),
    ];
    if distilled {
        lines.push(Line::styled(
            "This recording is already distilled. Point your agent at its skill in \
             ~/.agents/skills and ask it to perform the task.",
            theme::ok(),
        ));
    } else {
        lines.push(Line::styled(
            "This recording is not distilled yet. Press `d` to write a draft, refine it, \
             then install it with `galdr distill <id> --from <file>`.",
            theme::dim(),
        ));
    }
    Text::from(lines)
}

fn help_body() -> Text<'static> {
    Text::from(vec![
        Line::styled("Recordings", theme::ok()),
        Line::raw("  ↑↓ / j k   move selection"),
        Line::raw("  enter      open the inspector"),
        Line::raw("  d          distill a draft skill (galdr is the only writer)"),
        Line::raw("  a          open the audit / provenance view"),
        Line::raw("  r          what \"replay\" means"),
        Line::raw("  o          show the span path"),
        Line::raw(""),
        Line::styled("Inspector", theme::ok()),
        Line::raw("  ↑↓ / j k   move between steps"),
        Line::raw("  enter      show the raw tool_input / tool_response"),
        Line::raw("  esc / h    back to recordings"),
        Line::raw(""),
        Line::styled("Anywhere", theme::ok()),
        Line::raw("  ?          this help"),
        Line::raw("  q          quit"),
    ])
}

/// First 19 chars of an RFC3339 timestamp: `YYYY-MM-DDTHH:MM:SS`, dropping the
/// fractional seconds and zone for a compact display.
fn short_ts(ts: &str) -> String {
    ts.chars().take(19).collect()
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let vert = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .split(vert[1])[1]
}

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

    fn fixture() -> MockCatalog {
        let recording = RecordingRow {
            rec_id: "01AAA".into(),
            name: "tui demo".into(),
            started_at: "2026-06-19T10:00:00Z".into(),
            ended_at: Some("2026-06-19T10:05:00Z".into()),
            steps: 1,
            cwd: Some("/tmp".into()),
            distilled: true,
        };
        let detail = RecordingDetail {
            recording: recording.clone(),
            steps: vec![StepRow {
                seq: 0,
                tool_name: "Bash".into(),
                ts: "2026-06-19T10:00:01Z".into(),
                summary: "git status".into(),
            }],
        };
        MockCatalog {
            recordings: vec![recording],
            skills: vec![SkillRow {
                skill_name: "galdr-tui-demo".into(),
                rec_id: Some("01ZZZ".into()),
                skill_path: "/x/SKILL.md".into(),
                installed_at: None,
                orphan: true,
            }],
            detail: Some(detail),
            raw: vec![Event {
                ts: "2026-06-19T10:00:01Z".into(),
                seq: 0,
                tool_name: "Bash".into(),
                tool_input: serde_json::json!({ "command": "git status" }),
                tool_response: serde_json::json!({ "exit_code": 0 }),
                cwd: Some("/tmp".into()),
                session_id: None,
            }],
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    /// Renders the current state to a fixed-size test terminal and flattens the
    /// buffer to a string for substring assertions.
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
    fn renders_recordings_then_inspector_then_audit() {
        let mut app = App::new(fixture());

        let recordings = render_text(&mut app);
        assert!(recordings.contains("galdr"));
        assert!(recordings.contains("Recordings"));
        assert!(recordings.contains("tui demo"));

        // Enter opens the inspector for the selected recording.
        app.on_key(key(KeyCode::Enter));
        let inspector = render_text(&mut app);
        assert!(inspector.contains("Inspector"));
        assert!(inspector.contains("Steps"));
        assert!(inspector.contains("git status"));

        // Enter on a step opens the raw overlay with its warning and payload.
        app.on_key(key(KeyCode::Enter));
        let raw = render_text(&mut app);
        assert!(raw.contains("sensitive"));
        assert!(raw.contains("tool_input"));
        app.on_key(key(KeyCode::Esc)); // close overlay
        app.on_key(key(KeyCode::Esc)); // back to recordings

        // Audit shows the orphan skill.
        app.on_key(key(KeyCode::Char('a')));
        let audit = render_text(&mut app);
        assert!(audit.contains("galdr-tui-demo"));
        assert!(audit.contains("orphan"));
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
