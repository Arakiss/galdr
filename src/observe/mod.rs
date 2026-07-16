//! Human-observation recording lanes.
//!
//! `rec` records agent tool calls through harness hooks. `observe` is the second
//! sensor lane: human-demonstrated actions written as typed span events. The
//! synthetic fixture exercises storage and distillation deterministically; the
//! browser sensor uses loopback CDP to inject the local event collector into an
//! isolated Chrome profile.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};
use ulid::Ulid;

use crate::span::{
    Event, EventKind, HumanAction, HumanEvent, HumanSource, HumanTarget, HumanValue, TargetLocator,
};
use crate::{catalog, engine, ipc, paths, record, span, style};

include!("session.rs");
include!("cdp.rs");
include!("script.rs");
include!("process.rs");
include!("fixture.rs");
include!("tests.rs");
