//! Palette and styles for the TUI, derived for galdr (violet + teal on dark).

use ratatui::style::{Color, Modifier, Style};

/// galdr violet — the chant accent.
pub const ACCENT: Color = Color::Rgb(139, 123, 240);
/// Teal — local-first, the calm secondary.
pub const TEAL: Color = Color::Rgb(79, 214, 201);
/// Muted foreground for secondary text.
pub const DIM: Color = Color::Rgb(128, 128, 140);
/// Warning amber — raw data and orphans.
pub const WARN: Color = Color::Rgb(230, 160, 70);

pub fn title() -> Style {
    Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn selected() -> Style {
    Style::new()
        .bg(ACCENT)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::new().fg(DIM)
}

/// Primary foreground for normal text (the terminal's default ink).
pub fn text() -> Style {
    Style::new().fg(Color::Rgb(238, 236, 246))
}

/// Table-header style: mono-feeling, dim, uppercase handled by the caller.
pub fn header() -> Style {
    Style::new().fg(DIM).add_modifier(Modifier::BOLD)
}

pub fn warn() -> Style {
    Style::new().fg(WARN).add_modifier(Modifier::BOLD)
}

pub fn ok() -> Style {
    Style::new().fg(TEAL)
}
