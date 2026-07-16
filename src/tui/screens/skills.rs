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
        .highlight_symbol("▌ ")
        .block(block_focused(&format!("Harnesses · {detected}"), focused));
    frame.render_stateful_widget(table, area, &mut app.harness_state);
}
