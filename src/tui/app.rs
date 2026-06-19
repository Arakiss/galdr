//! TUI state and key handling, independent of rendering.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::data::Catalog;
use crate::catalog::{RecordingDetail, RecordingRow, SkillRow};
use crate::distill;
use crate::span::Event;

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
    pub recordings: Vec<RecordingRow>,
    pub skills: Vec<SkillRow>,
    pub rec_state: TableState,
    pub audit_state: TableState,
    pub detail: Option<RecordingDetail>,
    pub raw: Vec<Event>,
    pub detail_state: TableState,
    pub overlay: Option<Overlay>,
    pub status: String,
    pub should_quit: bool,
}

impl<C: Catalog> App<C> {
    pub fn new(catalog: C) -> Self {
        let recordings = catalog.recordings();
        let skills = catalog.skills();
        let mut rec_state = TableState::default();
        if !recordings.is_empty() {
            rec_state.select(Some(0));
        }
        let mut audit_state = TableState::default();
        if !skills.is_empty() {
            audit_state.select(Some(0));
        }
        Self {
            catalog,
            screen: Screen::Recordings,
            recordings,
            skills,
            rec_state,
            audit_state,
            detail: None,
            raw: Vec::new(),
            detail_state: TableState::default(),
            overlay: None,
            status: String::new(),
            should_quit: false,
        }
    }

    pub fn selected_recording(&self) -> Option<&RecordingRow> {
        self.rec_state
            .selected()
            .and_then(|i| self.recordings.get(i))
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if self.overlay.is_some() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                self.overlay = None;
            }
            return;
        }
        match self.screen {
            Screen::Recordings => self.on_key_recordings(key),
            Screen::Detail => self.on_key_detail(key),
            Screen::Audit => self.on_key_audit(key),
        }
    }

    fn on_key_recordings(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Char('j') | KeyCode::Down => {
                step(&mut self.rec_state, self.recordings.len(), 1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step(&mut self.rec_state, self.recordings.len(), -1);
            }
            KeyCode::Enter => self.open_detail(),
            KeyCode::Char('a') => self.open_audit(),
            KeyCode::Char('d') => self.distill_selected(),
            KeyCode::Char('o') => self.show_span_path(),
            KeyCode::Char('r') => self.overlay = Some(Overlay::Replay),
            _ => {}
        }
    }

    fn on_key_detail(&mut self, key: KeyEvent) {
        let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.screen = Screen::Recordings;
                self.detail = None;
                self.raw.clear();
                self.status.clear();
            }
            KeyCode::Char('j') | KeyCode::Down => step(&mut self.detail_state, steps, 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut self.detail_state, steps, -1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(i) = self.detail_state.selected() {
                    self.overlay = Some(Overlay::Raw(i));
                }
            }
            KeyCode::Char('o') => self.show_span_path(),
            _ => {}
        }
    }

    fn on_key_audit(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => self.screen = Screen::Recordings,
            KeyCode::Char('j') | KeyCode::Down => step(&mut self.audit_state, self.skills.len(), 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut self.audit_state, self.skills.len(), -1),
            _ => {}
        }
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
        if self.audit_state.selected().is_none() && !self.skills.is_empty() {
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
