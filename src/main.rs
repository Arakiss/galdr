//! galdr — local Record & Replay for agent skills.
//!
//! Records the tool calls a harness already emits (not pixels), stores them as an
//! append-only span, and distills them into a reproducible skill. Local-first: the
//! raw lives only in `~/.galdr` and nothing leaves the machine.

mod bench;
mod catalog;
mod config;
mod daemon;
mod diff;
mod distill;
mod doctor;
mod engine;
mod export;
mod ext;
mod harness;
mod hook;
mod ipc;
mod judge;
mod launchd;
mod link;
mod observe;
mod observe_mac;
mod outcome;
mod parametrize;
mod paths;
mod record;
mod regression;
mod remove;
mod setup;
mod skill;
mod span;
mod style;
mod suggest;
mod summary;
mod tui;
mod upgrade;
mod validate;

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

include!("main/cli.rs");
include!("main/dispatch.rs");
include!("main/status.rs");
include!("main/catalog.rs");
include!("main/evaluation.rs");
include!("main/validation.rs");
include!("main/output.rs");
include!("main/overview.rs");
