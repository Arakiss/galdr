//! Palette and styles for the TUI, derived for galdr (violet + teal on dark).
//!
//! One accent (violet, the chant) and one calm secondary (teal, local-first), on a
//! restrained gray ramp. Borders are dim by default and brighten to the accent on focus;
//! the selected row is a soft violet wash, not a hard reverse-video block.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::BorderType;

/// galdr violet — the chant accent.
pub const ACCENT: Color = Color::Rgb(139, 123, 240);
/// A brighter violet for the focused border and active tab — the accent, lit.
pub const ACCENT_BRIGHT: Color = Color::Rgb(167, 154, 255);
/// Teal — local-first, the calm secondary.
pub const TEAL: Color = Color::Rgb(79, 214, 201);
/// Muted foreground for secondary text.
pub const DIM: Color = Color::Rgb(122, 128, 148);
/// Fainter still — captions, resting separators.
pub const FAINT: Color = Color::Rgb(88, 94, 112);
/// Primary ink.
pub const INK: Color = Color::Rgb(234, 236, 246);
/// Warning amber — raw data and orphans.
pub const WARN: Color = Color::Rgb(230, 178, 94);
/// The soft wash behind the selected row.
pub const SELECT_BG: Color = Color::Rgb(46, 41, 78);

/// The border type every panel uses — rounded corners read softer than square.
pub const BORDER: BorderType = BorderType::Rounded;

pub fn title() -> Style {
    Style::new().fg(ACCENT_BRIGHT).add_modifier(Modifier::BOLD)
}

/// The selected row: a soft violet wash with bright ink, not hard reverse video.
pub fn selected() -> Style {
    Style::new()
        .bg(SELECT_BG)
        .fg(INK)
        .add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::new().fg(DIM)
}

/// The faintest text: resting separators and captions.
pub fn faint() -> Style {
    Style::new().fg(FAINT)
}

/// Primary foreground for normal text.
pub fn text() -> Style {
    Style::new().fg(INK)
}

/// The accent as plain foreground (no bold) — for keycaps and inline marks.
pub fn accent() -> Style {
    Style::new().fg(ACCENT)
}

/// Table-header style: dim and bold; the caller uppercases.
pub fn header() -> Style {
    Style::new()
        .fg(TEAL)
        .add_modifier(Modifier::BOLD | Modifier::DIM)
}

/// The filled pill behind the active tab.
pub fn tab_active() -> Style {
    Style::new()
        .bg(ACCENT)
        .fg(Color::Rgb(20, 18, 34))
        .add_modifier(Modifier::BOLD)
}

pub fn warn() -> Style {
    Style::new().fg(WARN).add_modifier(Modifier::BOLD)
}

pub fn ok() -> Style {
    Style::new().fg(TEAL)
}
