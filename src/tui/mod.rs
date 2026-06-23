//! Terminal UI: browse recordings, inspect a span, and audit skill provenance.
//!
//! The UI talks only to the [`data::Catalog`] trait; the default [`FsCatalog`]
//! reads `~/.galdr` straight from disk, so the TUI needs no daemon. A panic hook
//! restores the terminal before unwinding, so a bug never leaves a wrecked
//! terminal behind.

mod app;
mod data;
mod screens;
mod theme;

use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::App;
use data::FsCatalog;

/// Runs the TUI to completion (until the user quits).
pub fn run() -> Result<()> {
    let catalog = FsCatalog::new()?;
    let mut app = App::new(catalog);

    install_panic_hook();
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App<FsCatalog>) -> Result<()> {
    while !app.should_quit {
        // Cheap re-read of the active flag so the title's REC badge stays live.
        app.recording_active = crate::record::read_active().is_some();
        terminal.draw(|frame| screens::render(frame, app))?;
        // A timeout keeps the loop responsive to resizes even with no key input.
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.on_key(key);
        }
    }
    Ok(())
}

/// Chains a terminal restore in front of the existing panic hook, so an unexpected
/// panic anywhere in the UI still leaves the terminal usable.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        previous(info);
    }));
}
