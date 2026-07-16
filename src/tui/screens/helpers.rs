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
