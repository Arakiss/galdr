//! Data access for the TUI, behind a trait so the backing store can change.
//!
//! [`FsCatalog`] reads `~/.galdr` directly by building a throwaway in-memory index
//! from the spans and recordings on disk. It needs no daemon and no persistent
//! database. A daemon- or file-backed catalog can implement the same [`Catalog`]
//! trait and slot in without the UI noticing.
//!
//! The raw `tool_input` / `tool_response` are never in the index (it stores only
//! summaries). The inspector reads them straight from the span file on demand —
//! that is the single place a raw blob is ever surfaced.

use anyhow::Result;
use rusqlite::Connection;

use crate::catalog::{self, RecordingDetail, RecordingRow, SkillRow};
use crate::span::{self, Event};

pub trait Catalog {
    fn recordings(&self) -> Vec<RecordingRow>;
    fn detail(&self, rec_id: &str) -> Option<RecordingDetail>;
    fn raw_events(&self, rec_id: &str) -> Vec<Event>;
    fn skills(&self) -> Vec<SkillRow>;
    /// Reloads the backing store (e.g. after a distill). No-op by default.
    fn refresh(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct FsCatalog {
    conn: Connection,
}

impl FsCatalog {
    pub fn new() -> Result<Self> {
        Ok(Self {
            conn: catalog::open_in_memory_indexed()?,
        })
    }
}

impl Catalog for FsCatalog {
    fn recordings(&self) -> Vec<RecordingRow> {
        catalog::list_recordings(&self.conn).unwrap_or_default()
    }

    fn detail(&self, rec_id: &str) -> Option<RecordingDetail> {
        catalog::show_recording(&self.conn, rec_id).ok().flatten()
    }

    fn raw_events(&self, rec_id: &str) -> Vec<Event> {
        let Ok(path) = crate::paths::span_file(rec_id) else {
            return Vec::new();
        };
        span::read_span(&path).unwrap_or_default()
    }

    fn skills(&self) -> Vec<SkillRow> {
        catalog::list_skills(&self.conn).unwrap_or_default()
    }

    /// Rebuilds the in-memory index from disk.
    fn refresh(&mut self) -> Result<()> {
        self.conn = catalog::open_in_memory_indexed()?;
        Ok(())
    }
}
