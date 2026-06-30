//! TUI state and key handling, independent of rendering.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::data::Catalog;
use crate::catalog::{self, RecordingDetail, RecordingRow, SkillRow};
use crate::harness::{self, HarnessInfo};
use crate::{distill, record};

const PAGE: usize = 10;
const OVERLAY_PAGE: u16 = 12;

/// A focusable list panel in the sidebar. The preview pane follows the focused
/// panel's selection — the lazygit model: several lists at once, detail always visible.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Recordings,
    Skills,
    Harnesses,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::Recordings, Panel::Skills, Panel::Harnesses];

    fn index(self) -> usize {
        Panel::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn cycle(self, delta: isize) -> Panel {
        let len = Panel::ALL.len() as isize;
        let next = (self.index() as isize + delta).rem_euclid(len) as usize;
        Panel::ALL[next]
    }
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
    /// Which sidebar list is focused; the preview follows its selection.
    pub focus: Panel,
    /// True while the preview pane itself is focused (stepping through a recording's
    /// steps), so j/k move inside the preview instead of the sidebar list.
    pub preview_focus: bool,
    /// Rendered text of the selected skill's `SKILL.md`, shown in the preview.
    pub preview_md: String,

    pub recordings: Vec<RecordingRow>,
    pub skills: Vec<SkillRow>,
    pub harnesses: Vec<HarnessInfo>,
    /// Filtered/sorted projections actually shown; recomputed by [`Self::reproject`].
    pub rec_view: Vec<RecordingRow>,
    pub skill_view: Vec<SkillRow>,

    pub rec_state: TableState,
    pub skill_state: TableState,
    pub harness_state: TableState,
    pub detail: Option<RecordingDetail>,
    pub raw: Vec<crate::span::Event>,
    pub detail_state: TableState,

    pub overlay: Option<Overlay>,
    pub overlay_scroll: u16,
    pub filter: String,
    pub filter_mode: bool,

    pub recording_active: bool,
    pub active_name: Option<String>,
    pub status: String,
    pub should_quit: bool,
}

impl<C: Catalog> App<C> {
    pub fn new(catalog: C) -> Self {
        let recordings = catalog.recordings();
        let skills = catalog.skills();
        let mut app = Self {
            catalog,
            focus: Panel::Recordings,
            preview_focus: false,
            preview_md: String::new(),
            rec_view: recordings.clone(),
            skill_view: skills.clone(),
            recordings,
            skills,
            harnesses: harness::detect(),
            rec_state: TableState::default(),
            skill_state: TableState::default(),
            harness_state: TableState::default(),
            detail: None,
            raw: Vec::new(),
            detail_state: TableState::default(),
            overlay: None,
            overlay_scroll: 0,
            filter: String::new(),
            filter_mode: false,
            recording_active: false,
            active_name: None,
            status: String::new(),
            should_quit: false,
        };
        app.reproject();
        app.sync_preview();
        app
    }

    /// Number of galdr-distilled skills (vs external ones from other harnesses).
    pub fn galdr_skill_count(&self) -> usize {
        self.skills
            .iter()
            .filter(|s| s.origin == catalog::ORIGIN_GALDR)
            .count()
    }

    /// Recomputes the filtered views and keeps every selection valid. Skills are
    /// sorted galdr-first so a human sees their own distilled skills before the
    /// external ones that merely share the directory.
    pub fn reproject(&mut self) {
        let needle = self.filter.to_lowercase();
        let matches = |hay: &str| needle.is_empty() || hay.to_lowercase().contains(&needle);
        self.rec_view = self
            .recordings
            .iter()
            .filter(|r| matches(&r.name) || matches(&r.rec_id))
            .cloned()
            .collect();
        let mut skills: Vec<SkillRow> = self
            .skills
            .iter()
            .filter(|s| matches(&s.skill_name) || s.rec_id.as_deref().is_some_and(matches))
            .cloned()
            .collect();
        skills.sort_by(|a, b| {
            let a_ext = a.origin != catalog::ORIGIN_GALDR;
            let b_ext = b.origin != catalog::ORIGIN_GALDR;
            a_ext
                .cmp(&b_ext)
                .then_with(|| a.skill_name.cmp(&b.skill_name))
        });
        self.skill_view = skills;
        clamp_selection(&mut self.rec_state, self.rec_view.len());
        clamp_selection(&mut self.skill_state, self.skill_view.len());
        clamp_selection(&mut self.harness_state, self.harnesses.len());
    }

    pub fn selected_recording(&self) -> Option<&RecordingRow> {
        self.rec_state.selected().and_then(|i| self.rec_view.get(i))
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if self.filter_mode {
            return self.on_key_filter(key);
        }
        if self.overlay.is_some() {
            return self.on_key_overlay(key);
        }
        // Global keys available on every tab.
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('?') => return self.open_overlay(Overlay::Help),
            KeyCode::Tab => return self.switch_focus(self.focus.cycle(1)),
            KeyCode::BackTab => return self.switch_focus(self.focus.cycle(-1)),
            KeyCode::Char(c @ '1'..='3') => {
                let idx = c as usize - '1' as usize;
                return self.switch_focus(Panel::ALL[idx]);
            }
            _ => {}
        }
        // When the preview pane is focused, keys drive it (stepping through a recording).
        if self.preview_focus {
            return self.on_key_preview(key);
        }
        match self.focus {
            Panel::Recordings => self.on_key_recordings(key),
            Panel::Skills => self.on_key_skills(key),
            Panel::Harnesses => self.on_key_harnesses(key),
        }
    }

    fn switch_focus(&mut self, panel: Panel) {
        self.focus = panel;
        self.preview_focus = false;
        self.status.clear();
        self.sync_preview();
    }

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
                self.overlay_scroll = self.overlay_scroll.saturating_add(OVERLAY_PAGE)
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
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reproject();
                self.sync_preview();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                step(&mut self.rec_state, self.rec_view.len(), 1);
                self.sync_preview();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step(&mut self.rec_state, self.rec_view.len(), -1);
                self.sync_preview();
            }
            KeyCode::Char('g') | KeyCode::Home => {
                jump(&mut self.rec_state, self.rec_view.len(), true);
                self.sync_preview();
            }
            KeyCode::Char('G') | KeyCode::End => {
                jump(&mut self.rec_state, self.rec_view.len(), false);
                self.sync_preview();
            }
            KeyCode::PageDown => {
                page(&mut self.rec_state, self.rec_view.len(), PAGE as isize);
                self.sync_preview();
            }
            KeyCode::PageUp => {
                page(&mut self.rec_state, self.rec_view.len(), -(PAGE as isize));
                self.sync_preview();
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.enter_preview(),
            KeyCode::Char('d') => self.distill_selected(),
            KeyCode::Char('o') => self.show_span_path(),
            KeyCode::Char('r') => self.open_overlay(Overlay::Replay),
            _ => {}
        }
    }

    fn on_key_preview(&mut self, key: KeyEvent) {
        let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                // Leave the preview pane; the sidebar selection (and its live preview)
                // stays put.
                self.preview_focus = false;
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

    fn on_key_skills(&mut self, key: KeyEvent) {
        let len = self.skill_view.len();
        match key.code {
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reproject();
                self.sync_preview();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                step(&mut self.skill_state, len, 1);
                self.sync_preview();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step(&mut self.skill_state, len, -1);
                self.sync_preview();
            }
            KeyCode::Char('g') | KeyCode::Home => {
                jump(&mut self.skill_state, len, true);
                self.sync_preview();
            }
            KeyCode::Char('G') | KeyCode::End => {
                jump(&mut self.skill_state, len, false);
                self.sync_preview();
            }
            KeyCode::PageDown => {
                page(&mut self.skill_state, len, PAGE as isize);
                self.sync_preview();
            }
            KeyCode::PageUp => {
                page(&mut self.skill_state, len, -(PAGE as isize));
                self.sync_preview();
            }
            _ => {}
        }
    }

    fn on_key_harnesses(&mut self, key: KeyEvent) {
        let len = self.harnesses.len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => step(&mut self.harness_state, len, 1),
            KeyCode::Char('k') | KeyCode::Up => step(&mut self.harness_state, len, -1),
            KeyCode::Char('g') | KeyCode::Home => jump(&mut self.harness_state, len, true),
            KeyCode::Char('G') | KeyCode::End => jump(&mut self.harness_state, len, false),
            _ => {}
        }
    }

    /// One concrete skill, the selection in the Skills panel.
    fn selected_skill(&self) -> Option<&SkillRow> {
        self.skill_state
            .selected()
            .and_then(|i| self.skill_view.get(i))
    }

    /// Reprojects the preview pane onto the focused panel's current selection: a
    /// recording's steps, or a skill's `SKILL.md`. Cheap and called after every move.
    fn sync_preview(&mut self) {
        match self.focus {
            Panel::Recordings => {
                if let Some(rec) = self.selected_recording() {
                    let id = rec.rec_id.clone();
                    self.detail = self.catalog.detail(&id);
                    self.raw = self.catalog.raw_events(&id);
                } else {
                    self.detail = None;
                    self.raw.clear();
                }
                let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
                clamp_selection(&mut self.detail_state, steps);
                if steps > 0 && self.detail_state.selected().is_none() {
                    self.detail_state.select(Some(0));
                }
            }
            Panel::Skills => {
                self.preview_md = self
                    .selected_skill()
                    .and_then(|s| std::fs::read_to_string(&s.skill_path).ok())
                    .unwrap_or_default();
            }
            Panel::Harnesses => {}
        }
    }

    /// Focuses the preview pane to step through the selected recording.
    fn enter_preview(&mut self) {
        if self.focus != Panel::Recordings {
            return;
        }
        let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
        if steps == 0 {
            return;
        }
        if self.detail_state.selected().is_none() {
            self.detail_state.select(Some(0));
        }
        self.preview_focus = true;
    }

    fn open_overlay(&mut self, overlay: Overlay) {
        self.overlay = Some(overlay);
        self.overlay_scroll = 0;
    }

    /// Distills a complete skill for the selected recording through the sanctioned
    /// path — galdr stays the only writer of the skills directory.
    fn distill_selected(&mut self) {
        let Some(rec) = self.selected_recording() else {
            return;
        };
        let id = rec.rec_id.clone();
        match distill::distill(&id, None, false, false, None) {
            Ok(()) => {
                self.status =
                    format!("distilled {id} into a skill — now discoverable in your harnesses");
                let _ = self.catalog.refresh();
                self.recordings = self.catalog.recordings();
                self.skills = self.catalog.skills();
                self.reproject();
                self.sync_preview();
            }
            Err(err) => self.status = format!("distill failed: {err}"),
        }
    }

    fn show_span_path(&mut self) {
        let id = if self.preview_focus {
            self.detail.as_ref().map(|d| d.recording.rec_id.clone())
        } else {
            self.selected_recording().map(|r| r.rec_id.clone())
        };
        if let Some(id) = id
            && let Ok(path) = crate::paths::span_file(&id)
        {
            self.status = format!("span: {}", path.display());
        }
    }

    /// Re-reads the active-recording flag (called from the event loop) so the title
    /// REC badge stays live.
    pub fn refresh_active(&mut self) {
        match record::read_active() {
            Some(active) => {
                self.recording_active = true;
                self.active_name = Some(active.name);
            }
            None => {
                self.recording_active = false;
                self.active_name = None;
            }
        }
    }
}

fn step(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    state.select(Some((cur + delta).rem_euclid(len as isize) as usize));
}

fn page(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as isize;
    state.select(Some((cur + delta).clamp(0, len as isize - 1) as usize));
}

fn jump(state: &mut TableState, len: usize, top: bool) {
    if len == 0 {
        return;
    }
    state.select(Some(if top { 0 } else { len - 1 }));
}

fn clamp_selection(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
    } else {
        state.select(Some(state.selected().unwrap_or(0).min(len - 1)));
    }
}
