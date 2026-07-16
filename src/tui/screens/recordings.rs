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
    let judged_steps = detail
        .steps
        .iter()
        .filter(|step| !step.judgments.is_empty())
        .count();
    let fork_steps = detail
        .steps
        .iter()
        .filter(|step| {
            step.judgments
                .iter()
                .any(|judgment| judgment.verdict == "fork")
        })
        .count();

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
            Span::styled(
                if judged_steps > 0 {
                    format!("   ·   judgments: {judged_steps} ({fork_steps} fork)")
                } else {
                    "   ·   judgments: none".to_string()
                },
                if fork_steps > 0 {
                    theme::warn()
                } else {
                    theme::dim()
                },
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(meta).block(block("Inspector")), chunks[0]);

    let header = Row::new(["#", "tool", "summary"]).style(theme::header());
    let rows: Vec<Row> = detail
        .steps
        .iter()
        .map(|s| {
            let badge = if s
                .judgments
                .iter()
                .any(|judgment| judgment.verdict == "fork")
            {
                "[fork] "
            } else if s.judgments.iter().any(|judgment| judgment.verdict == "ok") {
                "[ok] "
            } else {
                ""
            };
            Row::new(vec![
                Cell::from((s.seq + 1).to_string()),
                Cell::from(Span::styled(s.tool_name.clone(), tool_style(&s.tool_name))),
                Cell::from(format!("{badge}{}", s.summary)),
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
        .highlight_symbol("▌ ")
        .block(block_focused(&steps_title, focused));
    frame.render_stateful_widget(table, chunks[1], &mut app.detail_state);
}
