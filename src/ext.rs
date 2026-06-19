//! Core extension points.
//!
//! galdr exposes two generic seams and nothing more: a **permission gate** that
//! decides whether an event may be recorded, and a **provenance sink** that
//! observes recorded events. The public core ships neutral implementations
//! (everything allowed, nothing recorded). Any concrete integration lives in a
//! layer outside this repository and plugs in by implementing these traits.

use crate::span::Event;

/// Decides whether an event should be recorded. Allows everything by default.
pub trait PermissionGate {
    /// `true` if the event may be recorded. An external layer can deny (for
    /// example, to exclude sensitive tools or paths).
    fn allow(&self, _event: &Event) -> bool {
        true
    }
}

/// Receives each recorded event. Does nothing by default.
pub trait ProvenanceSink {
    /// Notifies that an event was recorded. An external layer can annotate
    /// provenance, traceability, or audit.
    fn record(&self, _event: &Event) {}
}

/// Neutral implementation used by the public core: allows everything, records
/// nothing.
pub struct NoopExt;

impl PermissionGate for NoopExt {}
impl ProvenanceSink for NoopExt {}
