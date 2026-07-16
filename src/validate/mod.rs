//! The content gate every installed `SKILL.md` must pass.
//!
//! A galdr skill is executable: it is written into `~/.agents/skills`, linked into
//! Claude Code / Codex / Cursor where another agent reads and follows it, and shared
//! as OSS. The directory name is already guarded ([`crate::paths::skill_dir`]); this
//! gate guards the **content** along three orthogonal axes:
//!
//! - **Security** — secrets, personal/PII paths, an email, and dangerous commands.
//!   Secrets, PII and catastrophic commands *block* with no bypass; legitimately
//!   dangerous commands documented in prose only *warn* (galdr exists to capture real
//!   ops work, so blocking on a documented `git push --force` would defeat it).
//! - **Practicality** — the skill is actually a skill: complete anatomy, no empty
//!   required section, no leftover draft marker. Blocks. Skipped for drafts (a draft
//!   is scaffolding a human will finish, so its markers are expected).
//! - **Optimization** — a tautological description, noise steps, excessive length.
//!   Warns (and feeds the readiness score); blocks only under `--strict`.
//!
//! Severity is decided here; whether a `Warn` blocks is decided by the caller through
//! [`ValidationReport::has_blocking`] (true only under `--strict`). Drafts keep the
//! full Security axis — the file is one a human is about to open.

use std::fmt;

use crate::summary::slugify;

include!("types.rs");
include!("security.rs");
include!("practicality.rs");
include!("optimization.rs");
include!("generalize.rs");
include!("tests.rs");
