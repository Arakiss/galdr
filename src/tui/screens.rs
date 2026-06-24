//! Rendering for the tabs, the inspector, and the modal overlays.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::{App, Overlay, Tab};
use super::data::Catalog;
use super::theme;
use crate::catalog;

pub fn render<C: Catalog>(frame: &mut Frame, app: &mut App<C>) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1), // title strip
        Constraint::Length(1), // tab bar
        Constraint::Min(1),    // main
        Constraint::Length(1), // status / keybar
    ])
    .split(area);

    render_title(frame, chunks[0], app);
    render_tabbar(frame, chunks[1], app);
    if app.tab == Tab::Recordings && app.in_detail {
        render_detail(frame, chunks[2], app);
    } else {
        match app.tab {
            Tab::Overview => render_overview(frame, chunks[2], app),
            Tab::Recordings => render_recordings(frame, chunks[2], app),
            Tab::Skills => render_skills(frame, chunks[2], app),
            Tab::Harnesses => render_harnesses(frame, chunks[2], app),
        }
    }
    render_status(frame, chunks[3], app);

    if let Some(overlay) = app.overlay.as_ref() {
        render_overlay(frame, area, app, overlay);
    }
}

fn block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme::dim())
        .title(Span::styled(format!(" {title} "), theme::title()))
}

fn render_title<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let mut spans = vec![
        Span::styled("✦ galdr", theme::title()),
        Span::styled("  record & replay for agent skills", theme::dim()),
    ];
    if app.recording_active {
        spans.push(Span::styled("   ● REC ", theme::warn()));
        if let Some(name) = &app.active_name {
            spans.push(Span::styled(name.clone(), theme::warn()));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_tabbar<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let mut spans = Vec::new();
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, tab.title());
        if *tab == app.tab {
            spans.push(Span::styled(label, theme::selected()));
        } else {
            spans.push(Span::styled(label, theme::dim()));
        }
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_status<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    if app.filter_mode {
        let line = Line::from(vec![
            Span::styled("/", theme::title()),
            Span::styled(app.filter.clone(), theme::ok()),
            Span::styled("▏", theme::title()),
            Span::styled("   enter apply · esc clear", theme::dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let hints = if app.tab == Tab::Recordings && app.in_detail {
        "jk move · enter raw · o span · esc back · ? help"
    } else {
        match app.tab {
            Tab::Overview => "1-4/tab switch · enter recordings · ? help · q quit",
            Tab::Recordings => {
                "jk move · enter inspect · d distill · / filter · r replay · o span · ? help"
            }
            Tab::Skills => "jk move · / filter · 1-4/tab switch · ? help · q quit",
            Tab::Harnesses => "jk move · 1-4/tab switch · ? help · q quit",
        }
    };
    let mut spans = Vec::new();
    if !app.status.is_empty() {
        spans.push(Span::styled(format!("{}  ", app.status), theme::ok()));
    }
    if !app.filter.is_empty() {
        spans.push(Span::styled(
            format!("filter:{}  ", app.filter),
            theme::warn(),
        ));
    }
    spans.push(Span::styled(hints, theme::dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Overview ────────────────────────────────────────────────────────────────

fn render_overview<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let rows = Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).split(area);
    render_stat_cards(frame, rows[0], app);

    let cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(rows[1]);
    render_overview_harnesses(frame, cols[0], app);
    render_overview_recent(frame, cols[1], app);
}

fn render_stat_cards<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let detected = app.harnesses.iter().filter(|h| h.detected).count();
    let cards = [
        (
            format!("{}", app.recordings.len()),
            format!("recordings · {} distilled", app.distilled_count()),
            theme::ACCENT,
        ),
        (
            format!("{}", app.galdr_skill_count()),
            format!(
                "galdr skills · {} external",
                app.skills.len() - app.galdr_skill_count()
            ),
            theme::TEAL,
        ),
        (
            format!("{detected}/{}", app.harnesses.len()),
            "harnesses detected".to_string(),
            theme::ACCENT,
        ),
        (
            if app.recording_active {
                "● live".to_string()
            } else {
                "○ idle".to_string()
            },
            app.active_name
                .clone()
                .unwrap_or_else(|| "no active recording".to_string()),
            if app.recording_active {
                theme::WARN
            } else {
                theme::DIM
            },
        ),
    ];
    let slots = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(area);
    for (slot, (value, label, color)) in slots.iter().zip(cards) {
        let body = Text::from(vec![
            Line::raw(""),
            Line::styled(value, Style::new().fg(color)),
            Line::styled(label, theme::dim()),
        ]);
        frame.render_widget(
            Paragraph::new(body).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::dim()),
            ),
            *slot,
        );
    }
}

fn render_overview_harnesses<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let mut lines = Vec::new();
    for h in &app.harnesses {
        let (mark, style) = if h.detected {
            ("✓", theme::ok())
        } else {
            ("·", theme::dim())
        };
        let mut spans = vec![
            Span::styled(format!(" {mark} "), style),
            Span::styled(
                format!("{:<13}", h.name),
                if h.detected {
                    theme::text()
                } else {
                    theme::dim()
                },
            ),
        ];
        if let Some(true) = h.galdr_hook {
            spans.push(Span::styled("sensor wired", theme::ok()));
        } else if h.detected && h.galdr_hook == Some(false) {
            spans.push(Span::styled("sensor not wired", theme::warn()));
        } else if h.on_path {
            spans.push(Span::styled("on PATH", theme::dim()));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(
        Paragraph::new(lines).block(block("Harnesses on this system")),
        area,
    );
}

fn render_overview_recent<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let mut lines = Vec::new();
    lines.push(Line::styled(" recent recordings", theme::dim()));
    if app.recordings.is_empty() {
        lines.push(Line::styled(
            "   none yet — `galdr rec start <name>`",
            theme::dim(),
        ));
    }
    for rec in app.recordings.iter().take(5) {
        let mark = if rec.distilled { "✓" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!(" {mark} "), theme::ok()),
            Span::styled(format!("{:<22}", ellipsize(&rec.name, 22)), theme::text()),
            Span::styled(format!("{} steps", rec.steps), theme::dim()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(" galdr skills", theme::dim()));
    let galdr_skills: Vec<_> = app
        .skills
        .iter()
        .filter(|s| s.origin == catalog::ORIGIN_GALDR)
        .collect();
    if galdr_skills.is_empty() {
        lines.push(Line::styled(
            "   none yet — press `d` on a recording",
            theme::dim(),
        ));
    }
    for s in galdr_skills.iter().take(4) {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {:<24}", ellipsize(&s.skill_name, 24)),
                theme::ok(),
            ),
            Span::styled(
                format!("readiness {}", s.readiness_score),
                readiness_style(s.readiness_score),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).block(block("Recent activity")), area);
}

// ── Recordings ──────────────────────────────────────────────────────────────

fn render_recordings<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    if app.rec_view.is_empty() {
        let msg = if app.filter.is_empty() {
            "No recordings yet. Record one with `galdr rec start <name>`.".to_string()
        } else {
            format!(
                "No recordings match \"{}\". Press esc to clear the filter.",
                app.filter
            )
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(theme::dim())
                .block(block("Recordings")),
            area,
        );
        return;
    }
    let header =
        Row::new(["", "rec_id", "name", "steps", "recorded", "cwd"]).style(theme::header());
    let rows: Vec<Row> = app
        .rec_view
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
                Cell::from(ellipsize(&r.name, 24)),
                Cell::from(r.steps.to_string()),
                Cell::from(short_ts(&r.started_at)),
                Cell::from(basename(r.cwd.as_deref().unwrap_or("-"))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Length(26),
        Constraint::Length(24),
        Constraint::Length(5),
        Constraint::Length(19),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌ ")
        .block(block(&format!("Recordings · {}", app.rec_view.len())));
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
    let chunks = Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).split(area);

    let meta = vec![
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
        Line::from(vec![
            Span::styled(
                format!("distilled: {}", if rec.distilled { "yes" } else { "no" }),
                if rec.distilled {
                    theme::ok()
                } else {
                    theme::dim()
                },
            ),
            Span::styled(format!("   ·   {} steps", detail.steps.len()), theme::dim()),
            Span::styled(
                format!("   ·   cwd: {}", rec.cwd.as_deref().unwrap_or("-")),
                theme::dim(),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(meta).block(block("Inspector")), chunks[0]);

    let header = Row::new(["#", "tool", "summary"]).style(theme::header());
    let rows: Vec<Row> = detail
        .steps
        .iter()
        .map(|s| {
            Row::new(vec![
                Cell::from((s.seq + 1).to_string()),
                Cell::from(Span::styled(s.tool_name.clone(), tool_style(&s.tool_name))),
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

// ── Skills ──────────────────────────────────────────────────────────────────

fn render_skills<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    if app.skill_view.is_empty() {
        let msg = if app.filter.is_empty() {
            "No skills yet. Press `d` on a recording to distill a draft.".to_string()
        } else {
            format!(
                "No skills match \"{}\". Press esc to clear the filter.",
                app.filter
            )
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(theme::dim())
                .block(block("Skills")),
            area,
        );
        return;
    }
    let header =
        Row::new(["origin", "skill", "status", "readiness", "provenance"]).style(theme::header());
    let rows: Vec<Row> = app
        .skill_view
        .iter()
        .map(|s| {
            let is_galdr = s.origin == catalog::ORIGIN_GALDR;
            let origin = if is_galdr {
                Span::styled("galdr", theme::ok())
            } else {
                Span::styled("extern", theme::dim())
            };
            let name_style = if is_galdr {
                theme::text()
            } else {
                theme::dim()
            };
            let prov = match &s.rec_id {
                Some(id) if !s.orphan => format!("← {id}"),
                Some(id) => format!("← {id} (orphan)"),
                None => "—".to_string(),
            };
            Row::new(vec![
                Cell::from(origin),
                Cell::from(Span::styled(ellipsize(&s.skill_name, 30), name_style)),
                Cell::from(s.status.clone()),
                Cell::from(Span::styled(
                    format!("{}", s.readiness_score),
                    if is_galdr {
                        readiness_style(s.readiness_score)
                    } else {
                        theme::dim()
                    },
                )),
                Cell::from(Span::styled(prov, theme::dim())),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(6),
        Constraint::Length(30),
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Min(10),
    ];
    let g = app.galdr_skill_count();
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌ ")
        .block(block(&format!(
            "Skills · {g} galdr · {} external",
            app.skills.len() - g
        )));
    frame.render_stateful_widget(table, area, &mut app.skill_state);
}

// ── Harnesses ───────────────────────────────────────────────────────────────

fn render_harnesses<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    let header = Row::new(["", "harness", "config", "PATH", "galdr sensor"]).style(theme::header());
    let rows: Vec<Row> = app
        .harnesses
        .iter()
        .map(|h| {
            let (mark, mark_style) = if h.detected {
                ("✓", theme::ok())
            } else {
                ("·", theme::dim())
            };
            let name_style = if h.detected {
                theme::text()
            } else {
                theme::dim()
            };
            let sensor = match h.galdr_hook {
                Some(true) => Span::styled("wired", theme::ok()),
                Some(false) => Span::styled("not wired", theme::warn()),
                None => Span::styled("—", theme::dim()),
            };
            Row::new(vec![
                Cell::from(Span::styled(mark, mark_style)),
                Cell::from(Span::styled(h.name.clone(), name_style)),
                Cell::from(basename(h.config_dir.as_deref().unwrap_or("—"))),
                Cell::from(if h.on_path { "yes" } else { "—" }),
                Cell::from(sensor),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Length(16),
        Constraint::Length(18),
        Constraint::Length(6),
        Constraint::Min(10),
    ];
    let detected = app.harnesses.iter().filter(|h| h.detected).count();
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol("▌ ")
        .block(block(&format!("Harnesses · {detected} detected")));
    frame.render_stateful_widget(table, area, &mut app.harness_state);
}

// ── Overlays ────────────────────────────────────────────────────────────────

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
    let rect = centered(area, 82, 74);
    frame.render_widget(Clear, rect);
    let scrollable = matches!(overlay, Overlay::Raw(_));
    let foot = if scrollable {
        " jk/↑↓ scroll · g top · esc close "
    } else {
        " esc to close "
    };
    let blk = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {title} "),
            if warn { theme::warn() } else { theme::title() },
        ))
        .title_bottom(Span::styled(foot, theme::dim()));
    let para = Paragraph::new(body)
        .block(blk)
        .wrap(Wrap { trim: false })
        .scroll((app.overlay_scroll, 0));
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
        Line::styled("Navigation", theme::ok()),
        Line::raw("  1 2 3 4    jump to Overview / Recordings / Skills / Harnesses"),
        Line::raw("  tab        cycle tabs"),
        Line::raw("  ↑↓ / j k   move · g/G first/last · PgUp/PgDn page"),
        Line::raw("  /          filter (recordings & skills) · esc clears"),
        Line::raw(""),
        Line::styled("Recordings", theme::ok()),
        Line::raw("  enter      open the inspector"),
        Line::raw("  d          distill a draft skill (galdr is the only writer)"),
        Line::raw("  r          what \"replay\" means · o show the span path"),
        Line::raw(""),
        Line::styled("Inspector", theme::ok()),
        Line::raw("  enter      show the raw tool_input / tool_response (scrolls)"),
        Line::raw("  esc / h    back to the recordings list"),
        Line::raw(""),
        Line::styled("Anywhere", theme::ok()),
        Line::raw("  ?          this help · q quit"),
    ])
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn tool_style(tool: &str) -> Style {
    match tool {
        "Bash" => Style::new().fg(theme::ACCENT),
        "Read" | "Glob" | "Grep" => theme::ok(),
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => theme::warn(),
        _ => theme::dim(),
    }
}

fn readiness_style(score: i64) -> Style {
    if score >= 80 {
        theme::ok()
    } else {
        theme::warn()
    }
}

/// First 19 chars of an RFC3339 timestamp: `YYYY-MM-DDTHH:MM:SS`.
fn short_ts(ts: &str) -> String {
    ts.chars().take(19).collect()
}

fn ellipsize(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

fn basename(path: &str) -> String {
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
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
                ts: "2026-06-19T10:00:01Z".into(),
                summary: "git status".into(),
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
    fn overview_is_the_landing_tab_with_stats() {
        let mut app = App::new(fixture());
        let s = render_text(&mut app);
        assert!(s.contains("Overview"));
        assert!(s.contains("galdr skills"));
        // Harness panel always lists the known harnesses regardless of detection.
        assert!(s.contains("Claude Code"));
    }

    #[test]
    fn tabs_switch_to_recordings_skills_and_harnesses() {
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('2')));
        let recs = render_text(&mut app);
        assert!(recs.contains("Recordings"));
        assert!(recs.contains("tui demo"));

        app.on_key(key(KeyCode::Char('3')));
        let skills = render_text(&mut app);
        assert!(skills.contains("galdr-tui-demo"));
        assert!(skills.contains("extern")); // the bun skill is marked external

        app.on_key(key(KeyCode::Char('4')));
        let harn = render_text(&mut app);
        assert!(harn.contains("galdr sensor"));
        assert!(harn.contains("Claude Code"));
    }

    #[test]
    fn enter_opens_inspector_and_raw_overlay() {
        let mut app = App::new(fixture());
        app.on_key(key(KeyCode::Char('2'))); // Recordings
        app.on_key(key(KeyCode::Enter)); // inspector
        let insp = render_text(&mut app);
        assert!(insp.contains("Inspector"));
        assert!(insp.contains("Steps"));
        assert!(insp.contains("git status"));

        app.on_key(key(KeyCode::Enter)); // raw overlay
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
