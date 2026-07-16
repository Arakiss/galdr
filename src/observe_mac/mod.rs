//! Native macOS human-observation lane.
//!
//! The third sensor after `rec` (agent tool calls) and `observe browser` (CDP).
//! `observe mac` records a human demonstration of a GUI workflow on the Mac: clicks,
//! scrolls and keystrokes captured through a `CGEventTap` in listen-only mode, written
//! as typed human span events (`human.mac.*`). It mirrors the browser lane's shape —
//! a session file, a spawned sensor process, an events NDJSON that `stop` folds into
//! the immutable span — so distillation, the catalog and the daemon treat it like any
//! other recording.
//!
//! Phase 1 captures the action and its coordinates only. The accessibility context of
//! the clicked element (role, title, window) and optional screenshots are later phases;
//! the wire format already carries the fields so adding them needs no migration.
//!
//! Everything that touches macOS frameworks lives behind `#[cfg(target_os = "macos")]`
//! in [`sensor`]. On other platforms the sensor is a stub that fails with a clear
//! message, exactly as `launchd` install does off macOS.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::span::{
    Event, EventKind, HumanAction, HumanEvent, HumanSource, HumanTarget, HumanValue, TargetLocator,
};
use crate::{catalog, ipc, paths, record, span, style};

include!("session.rs");
include!("sensor.rs");
include!("tests.rs");
