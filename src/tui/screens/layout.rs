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
            spans.push(Span::styled(format!(" [ {label} ] "), theme::title()));
        } else {
            spans.push(Span::styled(format!("  {label}  "), theme::dim()));
        }
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
        theme::dim()
    };
    Block::default()
        .borders(Borders::ALL)
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
        .border_style(theme::dim())
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
    spans.push(Span::styled(hints, theme::dim()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
