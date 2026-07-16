//! Rendering for the sidebar panels, the live preview, and the modal overlays.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::{App, Overlay, Panel};
use super::data::Catalog;
use super::theme;
use crate::catalog;

include!("layout.rs");
include!("recordings.rs");
include!("skills.rs");
include!("overlays.rs");
include!("helpers.rs");
include!("tests.rs");
