//! TUI state and key handling, independent of rendering.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::widgets::TableState;

use super::data::Catalog;
use crate::catalog::{self, RecordingDetail, RecordingRow, SkillRow};
use crate::harness::{self, HarnessInfo};
use crate::{distill, export, link, outcome, paths, record, validate};

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
    /// Vertical scroll offset of the skill preview (when its pane is focused).
    pub preview_scroll: u16,

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
    /// How many recorded steps were hidden as setup/noise (galdr control commands,
    /// screenshots, throwaway reads) so the detail reads as the task, not its capture.
    pub hidden_steps: usize,
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
            preview_scroll: 0,
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
            hidden_steps: 0,
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
            KeyCode::Char('e') => self.export_selected(),
            KeyCode::Char('o') => self.show_span_path(),
            KeyCode::Char('r') => self.open_overlay(Overlay::Replay),
            _ => {}
        }
    }

    fn on_key_preview(&mut self, key: KeyEvent) {
        // esc/h/← always leaves the preview pane back to the sidebar.
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left) {
            self.preview_focus = false;
            self.status.clear();
            return;
        }
        // The Skills preview is scrolling text (a SKILL.md); the Recordings preview is a
        // step list with raw-payload drill-in.
        if self.focus == Panel::Skills {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.preview_scroll = self.preview_scroll.saturating_add(1)
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(1)
                }
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    self.preview_scroll = self.preview_scroll.saturating_add(PAGE as u16)
                }
                KeyCode::PageUp => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(PAGE as u16)
                }
                KeyCode::Char('g') | KeyCode::Home => self.preview_scroll = 0,
                _ => {}
            }
            return;
        }
        let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
        match key.code {
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
            KeyCode::Enter | KeyCode::Right => self.enter_preview(),
            KeyCode::Char('l') => self.link_selected(),
            KeyCode::Char('v') => self.validate_selected(),
            KeyCode::Char('O') => self.outcome_selected(),
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
                    let mut detail = self.catalog.detail(&id);
                    let raw_all = self.catalog.raw_events(&id);
                    // Hide setup/noise (galdr control commands, screenshots, throwaway
                    // reads) so the inspector reads as the task, not its capture — the
                    // same filter the distiller uses. Keep detail and raw aligned by seq,
                    // since the raw drill-in indexes them in parallel.
                    let meaningful = crate::distill::meaningful_steps(&raw_all);
                    let keep: std::collections::HashSet<u64> =
                        meaningful.iter().map(|e| e.seq).collect();
                    self.hidden_steps = if let Some(d) = detail.as_mut() {
                        let before = d.steps.len();
                        d.steps.retain(|s| keep.contains(&(s.seq as u64)));
                        before - d.steps.len()
                    } else {
                        0
                    };
                    self.detail = detail;
                    self.raw = meaningful;
                } else {
                    self.detail = None;
                    self.raw.clear();
                    self.hidden_steps = 0;
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
                self.preview_scroll = 0;
            }
            Panel::Harnesses => {}
        }
    }

    /// Focuses the preview pane: step through a recording's steps, or scroll a skill's
    /// `SKILL.md`.
    fn enter_preview(&mut self) {
        match self.focus {
            Panel::Recordings => {
                let steps = self.detail.as_ref().map_or(0, |d| d.steps.len());
                if steps == 0 {
                    return;
                }
                if self.detail_state.selected().is_none() {
                    self.detail_state.select(Some(0));
                }
                self.preview_focus = true;
            }
            Panel::Skills => {
                if self.preview_md.is_empty() {
                    return;
                }
                self.preview_scroll = 0;
                self.preview_focus = true;
            }
            Panel::Harnesses => {}
        }
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
        // The TUI is a human browsing: `d` installs the faithful render as a final,
        // discoverable skill (`--fast`). Authoring from the printed brief is the
        // agent/CLI path — its stdout would corrupt the full-screen UI anyway.
        match distill::distill(&id, None, true, false, None) {
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

    /// Links the selected skill into every installed harness (the same safe, local
    /// operation as `galdr link`).
    fn link_selected(&mut self) {
        let Some(skill) = self.selected_skill() else {
            return;
        };
        let name = skill.skill_name.clone();
        match link::link_skill(&name) {
            Ok(results) => {
                let reached = results
                    .iter()
                    .filter(|r| {
                        !matches!(
                            r.status,
                            link::LinkStatus::Conflict | link::LinkStatus::Failed
                        )
                    })
                    .count();
                self.status = format!("linked {name} into {reached} harness(es)");
                let _ = self.catalog.refresh();
            }
            Err(err) => self.status = format!("link failed: {err}"),
        }
    }

    /// Runs the install-time content gate over the selected skill and reports it.
    fn validate_selected(&mut self) {
        let Some(skill) = self.selected_skill() else {
            return;
        };
        let name = skill.skill_name.clone();
        let draft = matches!(
            skill.status.as_str(),
            catalog::STATUS_DRAFT | catalog::STATUS_PARAM_DRAFT
        );
        match std::fs::read_to_string(&skill.skill_path) {
            Ok(md) => {
                let ctx = validate::ValidationCtx::new(draft, false);
                let report = validate::validate_skill(&md, &ctx);
                self.status = if report.has_blocking(false) {
                    format!(
                        "⚠ {name}: {} blocking finding(s) — run `galdr validate {name}`",
                        report.blocking_count(false)
                    )
                } else {
                    format!("✓ {name} passes the content gate")
                };
            }
            Err(err) => self.status = format!("validate: cannot read {name}: {err}"),
        }
    }

    /// Records a success outcome for the selected skill against its provenance
    /// recording — the replay-reliability signal `galdr bench` reads.
    fn outcome_selected(&mut self) {
        let Some(skill) = self.selected_skill() else {
            return;
        };
        let name = skill.skill_name.clone();
        let Some(rec_id) = skill.rec_id.clone() else {
            self.status = format!("{name}: no provenance recording to attach an outcome to");
            return;
        };
        match outcome::record_usage(outcome::UsageInput {
            skill_name: name.clone(),
            rec_id,
            task_kind: None,
            outcome: "success".to_string(),
            retries: 0,
            manual_intervention_count: 0,
            notes: Some("recorded from the TUI".to_string()),
        }) {
            Ok(_) => self.status = format!("recorded a success outcome for {name}"),
            Err(err) => self.status = format!("outcome failed: {err}"),
        }
    }

    /// Exports the selected recording (metadata + summaries, no raw payloads) to a
    /// predictable directory under the galdr root.
    fn export_selected(&mut self) {
        let Some(rec) = self.selected_recording() else {
            return;
        };
        let id = rec.rec_id.clone();
        let Ok(out) = paths::galdr_root().map(|r| r.join("exports").join(&id)) else {
            self.status = "export: cannot resolve the galdr root".to_string();
            return;
        };
        match export::export_recording(&id, &out, false, false) {
            Ok(()) => self.status = format!("exported to {}", out.display()),
            Err(err) => self.status = format!("export failed: {err}"),
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
