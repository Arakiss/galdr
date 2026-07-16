//! Sensor: invoked by the harness after each tool call (PostToolUse).
//!
//! Contract: read the event from stdin and, if a recording is active, append it
//! to the span. Whatever happens, it must **never** break the agent session. The
//! robustness guard (catch panics, exit 0) lives in `main`; here the logic returns
//! `Result` and any error is discarded above.

use std::io::Read;

use anyhow::Result;
use serde::Deserialize;

use crate::ext::{PermissionGate, ProvenanceSink};
use crate::{config, ext, ipc, paths, record, span};

include!("run.rs");
include!("route.rs");
include!("capture.rs");
include!("tests.rs");
