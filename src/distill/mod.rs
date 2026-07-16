//! Distillation: span → `SKILL.md`.
//!
//! Every path shares one sanctioned writer ([`install_skill`]), so galdr stays the
//! only thing that writes the skills directory:
//!
//! - **Default (author):** galdr renders the faithful skill from the span (real steps,
//!   secrets redacted) but installs it as an unauthored *draft* and hands the agent an
//!   authoring brief: read the span, supply the judgment galdr can't (the why, the
//!   generalized inputs, the gotchas), then install with `--from`. galdr owns the
//!   mechanism; the agent owns the intelligence — the same split it uses for naming.
//! - **`--fast` (mechanical):** install that faithful render as a final skill in one
//!   step, no authoring pass. For a human or a headless run that wants the floor now.
//! - **`--auto` (autonomous):** a local MLX engine writes the finished `SKILL.md`
//!   from the span (untrusted-data delimiter, low temperature, output validated). If
//!   the engine is unavailable it falls back to the mechanical render as final.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::engine::{self, EngineKind};
use crate::span::{Event, HumanEvent, HumanValue, TargetLocator};
use crate::summary::{slugify, summarize_event};
use crate::{catalog, paths, record, span, validate};

include!("install.rs");
include!("prompt.rs");
include!("render.rs");
include!("tests.rs");
