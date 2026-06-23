//! TUI state and key handling, independent of rendering.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::data::Catalog;
use crate::catalog::{RecordingDetail, RecordingRow, SkillRow};
use crate::distill;
use crate::span::Event;

/// How many rows a PageUp/PageDown moves.
const PAGE: usize = 10;
/// How many lines a PageUp/PageDown scrolls inside an overlay.
const OVERLAY_PAGE: u16 = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Recordings,
    Detail,
    Audit,
}

/// A modal overlay drawn over the current screen.
pub enum Overlay {
    /// Raw `tool_input` / `tool_response` for the step at this index.
    Raw(usize),
    /// Honest explanation of what "replay" means in galdr.
    Replay,
    /// Keybinding help.
    Help,
}

pub struct App<C: Catalog> {
    pub catalog: C,
    pub screen: Screen,
    /// Every recording, newest first (the unfiltered source of truth).
    pub recordings: Vec<RecordingRow>,
    /// Every skill (the unfiltered source of truth).
    pub skills: Vec<SkillRow>,
    /// Filtered projections actually shown and navigated; recomputed by [`Self::reproject`].
    pub rec_view: Vec<RecordingRow>,
    pub skill_view: Vec<SkillRow>,
    pub rec_state: TableState,
    pub audit_state: TableState,
    pub detail: Option<RecordingDetail>,
    pub raw: Vec<Event>,
    pub detail_state: TableState,
    pub overlay: Option<Overlay>,
    /// Vertical scroll offset for the active overlay.
    pub overlay_scroll: u16,
    /// Substring filter applied to the recordings and audit lists.
    pub filter: String,
    /// True while the user is typing into the filter.
    pub filter_mode: bool,
    /// Whether a recording is currently being captured (shown in the title).
    pub recording_active: bool,
    pub status: String,
    pub should_quit: bool,
}

impl<C: Catalog> App<C> {
    pub fn new(catalog: C) -> Self {
        let recordings = catalog.recordings();
        let skills = catalog.skills();
        let mut app = Self {
            catalog,
            screen: Screen::Recordings,
            rec_view: recordings.clone(),
            skill_view: skills.clone(),
            recordings,
            skills,
            rec_state: TableState::default(),
            audit_state: TableState::default(),
            detail: None,
            raw: Vec::new(),
            detail_state: TableState::default(),
            overlay: None,
            overlay_scroll: 0,
            filter: String::new(),
            filter_mode: false,
            // Set live from the event loop; default false keeps `new` pure for tests.
            recording_active: false,
            status: String::new(),
            should_quit: false,
        };
        app.reproject();
        app
    }

    /// Recomputes the filtered views from `filter` and keeps each selection valid.
    pub fn reproject(&mut self) {
        let needle = self.filter.to_lowercase();
        let matches = |hay: &str| needle.is_empty() || hay.to_lowercase().contains(&needle);
        self.rec_view = self
            .recordings
            .iter()
            .filter(|r| matches(&r.name) || matches(&r.rec_id))
            .cloned()
            .collect();
        self.skill_view = self
            .skills
            .iter()
            .filter(|s| matches(&s.skill_name) || s.rec_id.as_deref().is_some_and(matches))
            .cloned()
            .collect();
        clamp_selection(&mut self.rec_state, self.rec_view.len());
        clamp_selection(&mut self.audit_state, self.skill_view.len());
    }

    pub fn selected_recording(&self) -> Option<&RecordingRow> {
        self.rec_state.selected().and_then(|i| self.rec_view.get(i))
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if self.filter_mode {
            self.on_key_filter(key);
            return;
        }
        if self.overlay.is_some() {
            self.on_key_overlay(key);
            return;
        }
        match self.screen {
            Screen::Recordings => self.on_key_recordings(key),
            Screen::Detail => self.on_key_detail(key),
            Screen::Audit => self.on_key_audit(key),
        }
    }

    /// Filter input: printable chars extend the needle, Backspace trims it, Enter
    /// keeps it and leaves input mode, Esc abandons and clears it.
    fn on_key_filter(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.filter_mode = false,
            KeyCode::Esc => {
                self.filter.clear();
                self.filter_mode = false;
                self.reproject();
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.reproject();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.reproject();
            }
            _ => {}
        }
    }

    fn on_key_overlay(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlay = None;
                self.overlay_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.overlay_scroll = self.overlay_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.overlay_scroll = self.overlay_scroll.saturating_sub(1)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.overlay_scroll = self.overlay_scroll.saturating_add(OVERLAY_PAGE);
            }
            KeyCode::PageUp => {
                self.overlay_scroll = self.overlay_scroll.saturating_sub(OVERLAY_PAGE)
            }
            KeyCode::Char('g') | KeyCode::Home => self.overlay_scroll = 0,
            _ => {}
        }
    }

    fn on_key_recordings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.open_overlay(Overlay::Help),
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reproject();
            }
            KeyCode::Char('j') | KeyCode::Down => step(&mut self.rec_state, self.rec_view.len(), 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut self.rec_state, self.rec_view.len(), -1),
            KeyCode::Char('g') | KeyCode::Home => {
                jump(&mut self.rec_state, self.rec_view.len(), true)
            }
            KeyCode::Char('G') | KeyCode::End => {
                jump(&mut self.rec_state, self.rec_view.len(), false)
            }
            KeyCode::PageDown => page(&mut self.rec_state, self.rec_view.len(), PAGE as isize),
            KeyCode::PageUp => page(&mut self.rec_state, self.rec_view.len(), -(PAGE as isize)),
            KeyCode::Enter => self.open_detail(),
            KeyCode::Char('a') => self.open_audit(),
            KeyCode::Char('d') => self.distill_selected(),
            KeyCode::Char('o') => self.show_span_path(),
            KeyCode::Char('r') => self.open_overlay(Overlay::Replay),
            _ => {}
        }
    }

    fn on_key_detail(&mut self, key: KeyEvent) {
        let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.open_overlay(Overlay::Help),
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.screen = Screen::Recordings;
                self.detail = None;
                self.raw.clear();
                self.status.clear();
            }
            KeyCode::Char('j') | KeyCode::Down => step(&mut self.detail_state, steps, 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut self.detail_state, steps, -1),
            KeyCode::Char('g') | KeyCode::Home => jump(&mut self.detail_state, steps, true),
            KeyCode::Char('G') | KeyCode::End => jump(&mut self.detail_state, steps, false),
            KeyCode::PageDown => page(&mut self.detail_state, steps, PAGE as isize),
            KeyCode::PageUp => page(&mut self.detail_state, steps, -(PAGE as isize)),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(i) = self.detail_state.selected() {
                    self.open_overlay(Overlay::Raw(i));
                }
            }
            KeyCode::Char('o') => self.show_span_path(),
            _ => {}
        }
    }

    fn on_key_audit(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.open_overlay(Overlay::Help),
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reproject();
            }
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => self.screen = Screen::Recordings,
            KeyCode::Char('j') | KeyCode::Down => {
                step(&mut self.audit_state, self.skill_view.len(), 1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step(&mut self.audit_state, self.skill_view.len(), -1)
            }
            KeyCode::Char('g') | KeyCode::Home => {
                jump(&mut self.audit_state, self.skill_view.len(), true)
            }
            KeyCode::Char('G') | KeyCode::End => {
                jump(&mut self.audit_state, self.skill_view.len(), false)
            }
            KeyCode::PageDown => page(&mut self.audit_state, self.skill_view.len(), PAGE as isize),
            KeyCode::PageUp => page(
                &mut self.audit_state,
                self.skill_view.len(),
                -(PAGE as isize),
            ),
            _ => {}
        }
    }

    fn open_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.overlay_scroll = 0;
    }

    fn open_detail(&mut self) {
        let Some(rec) = self.selected_recording() else {
            return;
        };
        let id = rec.rec_id.clone();
        self.detail = self.catalog.detail(&id);
        self.raw = self.catalog.raw_events(&id);
        self.detail_state = TableState::default();
        if self.detail.as_ref().is_some_and(|d| !d.steps.is_empty()) {
            self.detail_state.select(Some(0));
        }
        self.screen = Screen::Detail;
    }

    fn open_audit(&mut self) {
        if self.audit_state.selected().is_none() && !self.skill_view.is_empty() {
            self.audit_state.select(Some(0));
        }
        self.screen = Screen::Audit;
    }

    /// Distills a draft for the selected recording. galdr stays the only writer of
    /// the skills directory — this calls the sanctioned `distill` path.
    fn distill_selected(&mut self) {
        let Some(rec) = self.selected_recording() else {
            return;
        };
        let id = rec.rec_id.clone();
        match distill::distill(&id, None) {
            Ok(()) => {
                self.status = format!(
                    "draft written for {id} — refine it, then `galdr distill {id} --from <file>`"
                );
                let _ = self.catalog.refresh();
                self.recordings = self.catalog.recordings();
                self.skills = self.catalog.skills();
                self.reproject();
            }
            Err(err) => self.status = format!("distill failed: {err}"),
        }
    }

    fn show_span_path(&mut self) {
        let id = match self.screen {
            Screen::Detail => self.detail.as_ref().map(|d| d.recording.rec_id.clone()),
            _ => self.selected_recording().map(|r| r.rec_id.clone()),
        };
        if let Some(id) = id
            && let Ok(path) = crate::paths::span_file(&id)
        {
            self.status = format!("span: {}", path.display());
        }
    }
}

/// Moves a table selection by `delta`, wrapping around the ends.
fn step(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    let next = (cur + delta).rem_euclid(len as isize) as usize;
    state.select(Some(next));
}

/// Moves a table selection by `delta`, clamped to the ends (no wrap) — for paging.
fn page(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    let next = (cur + delta).clamp(0, len as isize - 1) as usize;
    state.select(Some(next));
}

/// Jumps to the first (`top`) or last row.
fn jump(state: &mut TableState, len: usize, top: bool) {
    if len == 0 {
        return;
    }
    state.select(Some(if top { 0 } else { len - 1 }));
}

/// Keeps a selection in range after the underlying list shrinks (e.g. a filter).
fn clamp_selection(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        let cur = state.selected().unwrap_or(0).min(len - 1);
        state.select(Some(cur));
    }
}
