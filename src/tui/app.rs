//! TUI state and key handling, independent of rendering.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::data::Catalog;
use crate::catalog::{self, RecordingDetail, RecordingRow, SkillRow};
use crate::harness::{self, HarnessInfo};
use crate::{distill, record};

const PAGE: usize = 10;
const OVERLAY_PAGE: u16 = 12;

/// The top-level tabs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Recordings,
    Skills,
    Harnesses,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Overview, Tab::Recordings, Tab::Skills, Tab::Harnesses];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Recordings => "Recordings",
            Tab::Skills => "Skills",
            Tab::Harnesses => "Harnesses",
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    fn cycle(self, delta: isize) -> Tab {
        let len = Tab::ALL.len() as isize;
        let next = (self.index() as isize + delta).rem_euclid(len) as usize;
        Tab::ALL[next]
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
    pub tab: Tab,
    /// True while the inspector (a recording's steps) is open over the Recordings tab.
    pub in_detail: bool,

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
            tab: Tab::Overview,
            in_detail: false,
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
        app
    }

    /// Number of galdr-distilled skills (vs external ones from other harnesses).
    pub fn galdr_skill_count(&self) -> usize {
        self.skills
            .iter()
            .filter(|s| s.origin == catalog::ORIGIN_GALDR)
            .count()
    }

    pub fn distilled_count(&self) -> usize {
        self.recordings.iter().filter(|r| r.distilled).count()
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
            KeyCode::Tab => return self.switch_tab(self.tab.cycle(1)),
            KeyCode::BackTab => return self.switch_tab(self.tab.cycle(-1)),
            KeyCode::Char(c @ '1'..='4') => {
                let idx = c as usize - '1' as usize;
                return self.switch_tab(Tab::ALL[idx]);
            }
            _ => {}
        }
        if self.tab == Tab::Recordings && self.in_detail {
            return self.on_key_detail(key);
        }
        match self.tab {
            Tab::Overview => self.on_key_overview(key),
            Tab::Recordings => self.on_key_recordings(key),
            Tab::Skills => self.on_key_skills(key),
            Tab::Harnesses => self.on_key_harnesses(key),
        }
    }

    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.in_detail = false;
        self.status.clear();
        // A filter is meaningful only on the list tabs; keep it but it simply has
        // no effect elsewhere.
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

    fn on_key_overview(&mut self, key: KeyEvent) {
        // The overview is a dashboard; Enter jumps into Recordings to act.
        if matches!(
            key.code,
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right
        ) {
            self.switch_tab(Tab::Recordings);
        }
    }

    fn on_key_recordings(&mut self, key: KeyEvent) {
        match key.code {
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
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.open_detail(),
            KeyCode::Char('d') => self.distill_selected(),
            KeyCode::Char('o') => self.show_span_path(),
            KeyCode::Char('r') => self.open_overlay(Overlay::Replay),
            _ => {}
        }
    }

    fn on_key_detail(&mut self, key: KeyEvent) {
        let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
        match key.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.in_detail = false;
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

    fn on_key_skills(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('/') => self.filter_mode = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.reproject();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                step(&mut self.skill_state, self.skill_view.len(), 1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step(&mut self.skill_state, self.skill_view.len(), -1)
            }
            KeyCode::Char('g') | KeyCode::Home => {
                jump(&mut self.skill_state, self.skill_view.len(), true)
            }
            KeyCode::Char('G') | KeyCode::End => {
                jump(&mut self.skill_state, self.skill_view.len(), false)
            }
            KeyCode::PageDown => page(&mut self.skill_state, self.skill_view.len(), PAGE as isize),
            KeyCode::PageUp => page(
                &mut self.skill_state,
                self.skill_view.len(),
                -(PAGE as isize),
            ),
            _ => {}
        }
    }

    fn on_key_harnesses(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                step(&mut self.harness_state, self.harnesses.len(), 1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step(&mut self.harness_state, self.harnesses.len(), -1)
            }
            KeyCode::Char('g') | KeyCode::Home => {
                jump(&mut self.harness_state, self.harnesses.len(), true)
            }
            KeyCode::Char('G') | KeyCode::End => {
                jump(&mut self.harness_state, self.harnesses.len(), false)
            }
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
        self.in_detail = true;
    }

    /// Distills a complete skill for the selected recording through the sanctioned
    /// path — galdr stays the only writer of the skills directory.
    fn distill_selected(&mut self) {
        let Some(rec) = self.selected_recording() else {
            return;
        };
        let id = rec.rec_id.clone();
        match distill::distill(&id, None, false, false) {
            Ok(()) => {
                self.status =
                    format!("distilled {id} into a skill — now discoverable in your harnesses");
                let _ = self.catalog.refresh();
                self.recordings = self.catalog.recordings();
                self.skills = self.catalog.skills();
                self.reproject();
            }
            Err(err) => self.status = format!("distill failed: {err}"),
        }
    }

    fn show_span_path(&mut self) {
        let id = if self.in_detail {
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
