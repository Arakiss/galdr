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
