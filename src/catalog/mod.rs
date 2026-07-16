//! Queryable catalog over the spans and recordings.
//!
//! The catalog is a SQLite **index, never the source of truth**. Everything in it
//! can be thrown away and rebuilt from `spans/` + `recordings/` (+ the skills
//! directory for provenance) with [`reindex`]. Two consequences follow from that
//! stance:
//!
//! - Blobs are never stored. A step keeps only a one-line `summary`
//!   ([`crate::summary::summarize_event`]), not the raw event payloads /
//!   `tool_response`. That keeps the index small and keeps sensitive payloads out
//!   of a second on-disk copy.
//! - The schema is created idempotently and gated by `PRAGMA user_version`, so an
//!   `open()` against an old or partial database just works.
//!
//! `recordings` carries a placeholder row (rec_id only) the moment a step is
//! indexed, so steps can be indexed before a recording is closed without breaking
//! the foreign key; the metadata is filled in on close.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, MAIN_DB, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::judge::StepJudgmentEvent;
use crate::outcome::{SkillOutcomeEvent, SkillUsageEvent};
use crate::record::Recording;
use crate::regression::RegressionBaseCaseEvent;
use crate::span::Event;

include!("types.rs");
include!("schema.rs");
include!("write.rs");
include!("query.rs");
include!("reindex.rs");
include!("skills.rs");
include!("tests.rs");
