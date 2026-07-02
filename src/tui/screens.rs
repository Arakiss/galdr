//! Rendering for the sidebar panels, the live preview, and the modal overlays.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::{App, Overlay, Panel};
use super::data::Catalog;
use super::theme;
use crate::catalog;

pub fn render<C: Catalog>(frame: &mut Frame, app: &mut App<C>) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1), // title: version + counts + recording status
        Constraint::Length(1), // tab bar
        Constraint::Min(1),    // body: the active tab
        Constraint::Length(1), // status / keybar
    ])
    .split(area);

    render_title(frame, chunks[0], app);
    render_tabbar(frame, chunks[1], app);
    render_body(frame, chunks[2], app);
    render_status(frame, chunks[3], app);

    if let Some(overlay) = app.overlay.as_ref() {
        render_overlay(frame, area, app, overlay);
    }
}

/// The tab bar, eldr-style: each tab numbered, the active one bracketed and accented.
fn render_tabbar<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let mut spans = Vec::new();
    for (i, tab) in Panel::ALL.iter().enumerate() {
        let label = format!("{} {}", i + 1, tab.label());
        if *tab == app.focus {
            spans.push(Span::styled(format!(" {label} "), theme::tab_active()));
        } else {
            spans.push(Span::styled(format!(" {label} "), theme::dim()));
        }
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The active tab. The Overview is a full-width dashboard; the others are a list on the
/// left and a live preview of the selection on the right.
fn render_body<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    if app.focus == Panel::Overview {
        render_overview(frame, area, app);
        return;
    }
    let cols =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);
    let on_list = !app.preview_focus;
    match app.focus {
        Panel::Recordings => {
            render_recordings(frame, cols[0], app, on_list);
            render_detail(frame, cols[1], app);
        }
        Panel::Skills => {
            render_skills(frame, cols[0], app, on_list);
            render_skill_preview(frame, cols[1], app);
        }
        Panel::Harnesses => {
            render_harnesses(frame, cols[0], app, on_list);
            render_harness_preview(frame, cols[1], app);
        }
        Panel::Overview => {}
    }
}

/// A block whose border turns accent when its panel holds focus.
fn block_focused(title: &str, focused: bool) -> Block<'static> {
    let border = if focused {
        theme::title()
    } else {
        theme::faint()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(theme::BORDER)
        .border_style(border)
        .title(Span::styled(format!(" {title} "), theme::title()))
}

/// Preview pane for the Skills panel: the selected skill's `SKILL.md`.
fn render_skill_preview<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let name = app
        .skill_view
        .get(app.skill_state.selected().unwrap_or(0))
        .map(|s| s.skill_name.as_str())
        .unwrap_or("—");
    let body = if app.preview_md.is_empty() {
        Paragraph::new("(select a skill to preview its SKILL.md)").style(theme::dim())
    } else {
        Paragraph::new(app.preview_md.clone())
            .style(theme::text())
            .wrap(Wrap { trim: false })
            .scroll((app.preview_scroll, 0))
    };
    let title = if app.focus == Panel::Skills && app.preview_focus {
        format!("Skill · {name} · jk scroll")
    } else {
        format!("Skill · {name}")
    };
    frame.render_widget(body.block(block_focused(&title, app.preview_focus)), area);
}

/// Preview pane for the Harnesses panel: the selected harness, in detail.
fn render_harness_preview<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let Some(h) = app.harnesses.get(app.harness_state.selected().unwrap_or(0)) else {
        frame.render_widget(Paragraph::new("(no harness)").block(block("Harness")), area);
        return;
    };
    let sensor = match h.galdr_hook {
        Some(true) => Span::styled("galdr sensor wired", theme::ok()),
        Some(false) => Span::styled("galdr sensor not wired — run `galdr setup`", theme::warn()),
        None => Span::styled("galdr sensor: n/a", theme::dim()),
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                h.name.clone(),
                if h.detected {
                    theme::title()
                } else {
                    theme::dim()
                },
            ),
            Span::styled(
                if h.detected {
                    "  detected"
                } else {
                    "  not installed"
                },
                theme::dim(),
            ),
        ]),
        Line::styled(
            format!("config: {}", h.config_dir.as_deref().unwrap_or("—")),
            theme::dim(),
        ),
        Line::styled(
            format!("on PATH: {}", if h.on_path { "yes" } else { "no" }),
            theme::dim(),
        ),
        Line::raw(""),
        Line::from(sensor),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block(&format!("Harness · {}", h.name))),
        area,
    );
}

fn block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(theme::BORDER)
        .border_style(theme::faint())
        .title(Span::styled(format!(" {title} "), theme::title()))
}

fn render_title<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let left = Line::from(vec![
        Span::styled(
            concat!("✦ galdr v", env!("CARGO_PKG_VERSION")),
            theme::title(),
        ),
        Span::styled(
            format!(
                "   ·   {} recordings   ·   {} galdr · {} ext skills",
                app.recordings.len(),
                app.galdr_skill_count(),
                app.external_skill_count()
            ),
            theme::dim(),
        ),
    ]);
    let right = if app.recording_active {
        Line::from(Span::styled(
            format!("● REC {} ", app.active_name.as_deref().unwrap_or("")),
            theme::warn(),
        ))
    } else {
        Line::from(Span::styled("● idle ", theme::dim()))
    };
    let cols =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).split(area);
    frame.render_widget(Paragraph::new(left), cols[0]);
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), cols[1]);
}

/// The Overview tab: a dashboard of what galdr has and what needs attention, so a human
/// opening the TUI is oriented at a glance before drilling into any list.
fn render_overview<C: Catalog>(frame: &mut Frame, area: Rect, app: &App<C>) {
    let mut lines: Vec<Line> = Vec::new();

    // The surfaced insight line: what is actionable right now (eldr's ⚠ line).
    let drafts = app.draft_count();
    let undistilled = app.undistilled_count();
    if drafts > 0 || undistilled > 0 {
        let mut parts = Vec::new();
        if drafts > 0 {
            parts.push(format!("{drafts} skill(s) await authoring"));
        }
        if undistilled > 0 {
            parts.push(format!("{undistilled} recording(s) not yet a skill"));
        }
        lines.push(Line::styled(
            format!("⚠ {}", parts.join("   ·   ")),
            theme::warn(),
        ));
    } else {
        lines.push(Line::styled(
            "✓ every recording distilled and authored",
            theme::ok(),
        ));
    }
    lines.push(Line::raw(""));

    // Big-number stats.
    let ready = app
        .avg_readiness()
        .map(|r| r.to_string())
        .unwrap_or_else(|| "—".into());
    lines.push(Line::from(vec![
        Span::styled("SKILLS ", theme::dim()),
        Span::styled(app.skills.len().to_string(), theme::title()),
        Span::styled(
            format!(
                "  {} galdr · {} ext",
                app.galdr_skill_count(),
                app.external_skill_count()
            ),
            theme::dim(),
        ),
        Span::styled("      READY ", theme::dim()),
        Span::styled(ready, theme::title()),
        Span::styled("      RECORDINGS ", theme::dim()),
        Span::styled(app.recordings.len().to_string(), theme::title()),
    ]));
    lines.push(Line::raw(""));

    // Recent recordings.
    lines.push(Line::styled("Recent recordings", theme::ok()));
    if app.recordings.is_empty() {
        lines.push(Line::styled(
            "  (none yet — `galdr rec start <name>`)",
            theme::dim(),
        ));
    }
    for r in app.recordings.iter().take(5) {
        let (mark, mstyle) = if r.distilled {
            ("✓", theme::ok())
        } else {
            ("·", theme::dim())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {mark} "), mstyle),
            Span::styled(format!("{:<26}", ellipsize(&r.name, 26)), theme::text()),
            Span::styled(short_ts(&r.started_at), theme::dim()),
        ]));
    }
    lines.push(Line::raw(""));

    // Your distilled skills.
    lines.push(Line::styled("Your skills", theme::ok()));
    let galdr_skills: Vec<_> = app
        .skills
        .iter()
        .filter(|s| s.origin == catalog::ORIGIN_GALDR)
        .collect();
    if galdr_skills.is_empty() {
        lines.push(Line::styled(
            "  (none yet — distill a recording on the Recordings tab)",
            theme::dim(),
        ));
    }
    for s in galdr_skills.iter().take(5) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<26}", ellipsize(&s.skill_name, 26)),
                theme::text(),
            ),
            Span::styled(
                format!("{:<7}", s.status),
                readiness_style(s.readiness_score),
            ),
            Span::styled(format!("rdy {}", s.readiness_score), theme::dim()),
        ]));
    }
    lines.push(Line::raw(""));

    // Harnesses and their sensor wiring.
    let mut hspans = vec![Span::styled("Harnesses   ", theme::dim())];
    for h in app.harnesses.iter().filter(|h| h.detected) {
        let (mark, st) = match h.galdr_hook {
            Some(true) => ("✓", theme::ok()),
            Some(false) => ("✗", theme::warn()),
            None => ("·", theme::dim()),
        };
        hspans.push(Span::styled(format!("{mark} {}    ", h.name), st));
    }
    lines.push(Line::from(hspans));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Tab or 1–4 move between tabs · Enter browse recordings · ? help · q quit",
        theme::dim(),
    ));

    frame.render_widget(Paragraph::new(lines).block(block("Overview")), area);
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
    let hints = if app.preview_focus {
        match app.focus {
            Panel::Skills => "jk scroll · g top · esc back · ? help",
            _ => "jk step · enter raw · o span · esc back · ? help",
        }
    } else {
        match app.focus {
            Panel::Overview => "tab/1-4 move · enter browse recordings · ? help · q quit",
            Panel::Recordings => {
                "jk move · enter inspect · d distill · e export · / filter · r replay · tab · ?"
            }
            Panel::Skills => {
                "jk move · enter read · l link · v validate · O outcome · / filter · tab · ?"
            }
            Panel::Harnesses => "jk move · tab/1-4 tab · ? help · q quit",
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
    spans.extend(styled_hints(hints));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Renders a `"key action · key action"` keybar with the key of each segment accented
/// and the rest dim, so the shortcuts read at a glance instead of a flat gray run.
fn styled_hints(hints: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, seg) in hints.split(" · ").enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", theme::faint()));
        }
        match seg.split_once(' ') {
            Some((key, rest)) => {
                spans.push(Span::styled(key.to_string(), theme::accent()));
                spans.push(Span::styled(format!(" {rest}"), theme::dim()));
            }
            None => spans.push(Span::styled(seg.to_string(), theme::accent())),
        }
    }
    spans
}

// ── Recordings ──────────────────────────────────────────────────────────────

fn render_recordings<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>, focused: bool) {
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
                .block(block_focused("Recordings", focused)),
            area,
        );
        return;
    }
    let header = Row::new(["", "rec_id", "name", "steps"]).style(theme::header());
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
                Cell::from(Span::styled(short_id(&r.rec_id), theme::dim())),
                Cell::from(ellipsize(&r.name, 22)),
                Cell::from(r.steps.to_string()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Min(12),
        Constraint::Length(5),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol(theme::CURSOR)
        .highlight_symbol("▌ ")
        .block(block_focused(
            &format!("Recordings · {}", app.rec_view.len()),
            focused,
        ));
    frame.render_stateful_widget(table, area, &mut app.rec_state);
}

fn render_detail<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>) {
    let hidden = app.hidden_steps;
    let focused = app.preview_focus;
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
            Span::styled(
                if hidden > 0 {
                    format!(
                        "   ·   {} steps (+{hidden} setup hidden)",
                        detail.steps.len()
                    )
                } else {
                    format!("   ·   {} steps", detail.steps.len())
                },
                theme::dim(),
            ),
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
    let steps_title = if hidden > 0 {
        format!("Steps · {hidden} setup/noise hidden")
    } else {
        "Steps".to_string()
    };
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol(theme::CURSOR)
        .highlight_symbol("▌ ")
        .block(block_focused(&steps_title, focused));
    frame.render_stateful_widget(table, chunks[1], &mut app.detail_state);
}

// ── Skills ──────────────────────────────────────────────────────────────────

fn render_skills<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>, focused: bool) {
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
                .block(block_focused("Skills", focused)),
            area,
        );
        return;
    }
    let header = Row::new(["origin", "skill", "rdy"]).style(theme::header());
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
            Row::new(vec![
                Cell::from(origin),
                Cell::from(Span::styled(ellipsize(&s.skill_name, 26), name_style)),
                Cell::from(Span::styled(
                    format!("{}", s.readiness_score),
                    if is_galdr {
                        readiness_style(s.readiness_score)
                    } else {
                        theme::dim()
                    },
                )),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(6),
        Constraint::Min(12),
        Constraint::Length(3),
    ];
    let g = app.galdr_skill_count();
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol(theme::CURSOR)
        .highlight_symbol("▌ ")
        .block(block_focused(
            &format!("Skills · {g} galdr · {} ext", app.skills.len() - g),
            focused,
        ));
    frame.render_stateful_widget(table, area, &mut app.skill_state);
}

// ── Harnesses ───────────────────────────────────────────────────────────────

fn render_harnesses<C: Catalog>(frame: &mut Frame, area: Rect, app: &mut App<C>, focused: bool) {
    let header = Row::new(["", "harness", "sensor"]).style(theme::header());
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
                Cell::from(sensor),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(9),
    ];
    let detected = app.harnesses.iter().filter(|h| h.detected).count();
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected())
        .highlight_symbol(theme::CURSOR)
        .highlight_symbol("▌ ")
        .block(block_focused(&format!("Harnesses · {detected}"), focused));
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
        Line::styled("Panels", theme::ok()),
        Line::raw("  1 2 3      focus Recordings / Skills / Harnesses"),
        Line::raw("  tab        cycle the focused panel · the preview follows the selection"),
        Line::raw("  ↑↓ / j k   move · g/G first/last · PgUp/PgDn page"),
        Line::raw("  /          filter (recordings & skills) · esc clears"),
        Line::raw(""),
        Line::styled("Recordings", theme::ok()),
        Line::raw("  enter      step into the preview (inspect the steps)"),
        Line::raw("  d          distill a complete skill (galdr is the only writer)"),
        Line::raw("  e          export this recording (no raw payloads)"),
        Line::raw("  r          what \"replay\" means · o show the span path"),
        Line::raw(""),
        Line::styled("Skills", theme::ok()),
        Line::raw("  enter      read the SKILL.md (jk scroll · esc back)"),
        Line::raw("  l          link into every installed harness"),
        Line::raw("  v          validate against the content gate"),
        Line::raw("  O          record a success outcome (feeds `galdr bench`)"),
        Line::raw(""),
        Line::styled("Preview (a recording's steps)", theme::ok()),
        Line::raw("  enter      show the raw tool_input / tool_response (scrolls)"),
        Line::raw("  esc / h    back to the sidebar"),
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

/// A compact reference for a ULID rec_id in the narrow sidebar: its last 6 characters
/// (the random tail), enough to recognize and disambiguate at a glance.
fn short_id(id: &str) -> String {
    let n = id.chars().count();
    id.chars().skip(n.saturating_sub(6)).collect()
}

fn ellipsize(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
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
                event_kind: "tool_call".into(),
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
                },
                StepRow {
                    seq: 1,
                    tool_name: "Bash".into(),
                    event_kind: "tool_call".into(),
                    ts: "t".into(),
                    summary: "galdr rec status".into(),
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
