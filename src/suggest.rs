//! Skill-opportunity detection: turn "you've done this before" into a signal.
//!
//! The reminder hook nudges on keywords; this scores the actual recordings. A task
//! repeated across recordings with the same structural shape — and not yet distilled
//! into a skill — is an opportunity worth crystallizing. `suggest` reads the catalog,
//! signs every recording by the sequence of its meaningful steps (the same
//! [`crate::diff::shape_key`] the diff aligns on), groups runs that share a shape,
//! **dedupes against the skills already installed** (a recording whose shape was
//! distilled is covered), and ranks what is left by repeatability. It moves "skill
//! opportunity" from the agent's judgment to a number the system can report.
//!
//! It only sees recorded sessions, so "done N times" means recorded N times with the
//! same shape — record tasks as you do them and the repeated ones surface here.

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;

use crate::{catalog, diff, distill, paths, span};

/// One recording reduced to the inputs the detector needs.
struct RecShape {
    rec_id: String,
    name: String,
    started_at: String,
    /// The sequence of meaningful-step shape keys, joined — the recording's signature.
    signature: String,
    /// Number of meaningful steps behind the signature.
    steps: usize,
    /// `true` if a skill was already distilled from this recording.
    distilled: bool,
}

/// One recording inside an opportunity group.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OppRec {
    pub rec_id: String,
    pub name: String,
    pub started_at: String,
}

/// A detected opportunity: recordings that share a structural shape and are not yet
/// covered by an installed skill.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Opportunity {
    /// How many recordings share this shape (the repeatability count).
    pub count: usize,
    /// Number of meaningful steps in the shape.
    pub steps: usize,
    /// Repeatability score: repeatability dominates, step count breaks ties.
    pub score: u32,
    /// Recordings in this group, most recent first.
    pub recordings: Vec<OppRec>,
    /// One-line, human-facing reason and next action.
    pub recommendation: String,
}

/// Detects opportunities from a set of recording shapes. Pure (no IO) so it is unit
/// tested directly; `run` supplies the shapes from disk.
fn detect(shapes: Vec<RecShape>, min_count: usize) -> Vec<Opportunity> {
    // Group recordings by their structural signature.
    let mut groups: HashMap<String, Vec<RecShape>> = HashMap::new();
    for shape in shapes {
        if shape.signature.is_empty() {
            continue; // a recording with no meaningful steps signs nothing
        }
        groups
            .entry(shape.signature.clone())
            .or_default()
            .push(shape);
    }

    let mut out: Vec<Opportunity> = Vec::new();
    for (_signature, mut members) in groups {
        // Dedupe against installed skills: if any recording of this shape was already
        // distilled, the shape is covered — not an opportunity.
        if members.iter().any(|m| m.distilled) {
            continue;
        }
        let count = members.len();
        if count < min_count {
            continue;
        }
        // Most recent first, so the freshest recording leads the next action.
        members.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        let steps = members[0].steps;
        let score = (count as u32) * 100 + (steps.min(20) as u32);
        let recommendation = recommend(&members);
        out.push(Opportunity {
            count,
            steps,
            score,
            recordings: members
                .into_iter()
                .map(|m| OppRec {
                    rec_id: m.rec_id,
                    name: m.name,
                    started_at: m.started_at,
                })
                .collect(),
            recommendation,
        });
    }

    // Rank: repeatability first, then step count, then a stable tiebreak on rec_id.
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.recordings[0].rec_id.cmp(&b.recordings[0].rec_id))
    });
    out
}

/// The next-action sentence for a group.
fn recommend(members: &[RecShape]) -> String {
    let newest = &members[0].rec_id;
    if members.len() >= 2 {
        format!(
            "Recorded {}× and not a skill yet — `galdr distill {}`, or record it once more and `galdr parametrize {} {}` to name the inputs that vary.",
            members.len(),
            newest,
            members[1].rec_id,
            newest,
        )
    } else {
        format!("Recorded once and not distilled — `galdr distill {newest}` to crystallize it.")
    }
}

/// Loads every recording's shape from the catalog and the spans on disk.
fn load_shapes() -> Result<Vec<RecShape>> {
    let conn = catalog::open_in_memory_indexed()?;
    let mut shapes = Vec::new();
    for row in catalog::list_recordings(&conn)? {
        let signature = match paths::span_file(&row.rec_id) {
            Ok(path) => signature_for(&path),
            Err(_) => String::new(),
        };
        let steps = if signature.is_empty() {
            0
        } else {
            signature.split('\n').count()
        };
        shapes.push(RecShape {
            rec_id: row.rec_id,
            name: row.name,
            started_at: row.started_at,
            signature,
            steps,
            distilled: row.distilled,
        });
    }
    Ok(shapes)
}

/// Signs a recording: the meaningful steps' shape keys, joined by newline. Empty when
/// the span is unreadable or carries no meaningful step.
fn signature_for(span_path: &std::path::Path) -> String {
    let events = match span::read_span(span_path) {
        Ok(events) => events,
        Err(_) => return String::new(),
    };
    distill::meaningful_steps(&events)
        .iter()
        .map(diff::shape_key)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs the detector and prints the result (human table or `--json`).
pub fn run(json: bool, top: Option<usize>, min_count: usize) -> Result<()> {
    let shapes = load_shapes()?;
    let mut opportunities = detect(shapes, min_count.max(1));
    if let Some(top) = top {
        opportunities.truncate(top);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&opportunities)?);
        return Ok(());
    }

    if opportunities.is_empty() {
        println!(
            "No repeated, uncaptured tasks found (min count {}).",
            min_count.max(1)
        );
        println!(
            "Record tasks as you do them (`galdr rec start <slug>`); this surfaces the shapes worth turning into a skill."
        );
        return Ok(());
    }

    println!("Skill opportunities — repeated tasks not yet distilled:\n");
    for (i, opp) in opportunities.iter().enumerate() {
        let lead = &opp.recordings[0];
        println!(
            "{:>2}. {} ×{}  ({} step{})  score {}",
            i + 1,
            lead.name,
            opp.count,
            opp.steps,
            if opp.steps == 1 { "" } else { "s" },
            opp.score,
        );
        println!("    {}", opp.recommendation);
        if opp.count > 1 {
            let others: Vec<&str> = opp.recordings.iter().map(|r| r.rec_id.as_str()).collect();
            println!("    recordings: {}", others.join(", "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(rec_id: &str, sig: &str, started: &str, distilled: bool) -> RecShape {
        RecShape {
            rec_id: rec_id.into(),
            name: format!("task-{rec_id}"),
            started_at: started.into(),
            signature: sig.into(),
            steps: sig.split('\n').count(),
            distilled,
        }
    }

    #[test]
    fn repeated_uncaptured_shape_is_an_opportunity() {
        let shapes = vec![
            shape(
                "a",
                "Bash:command|grep\nWrite:file_path",
                "2026-01-01",
                false,
            ),
            shape(
                "b",
                "Bash:command|grep\nWrite:file_path",
                "2026-01-02",
                false,
            ),
        ];
        let opps = detect(shapes, 2);
        assert_eq!(opps.len(), 1);
        assert_eq!(opps[0].count, 2);
        // Most recent recording leads the recommendation.
        assert_eq!(opps[0].recordings[0].rec_id, "b");
        assert!(opps[0].recommendation.contains("galdr distill b"));
        assert!(opps[0].recommendation.contains("galdr parametrize a b"));
    }

    #[test]
    fn an_already_distilled_shape_is_deduped_out() {
        let shapes = vec![
            shape(
                "a",
                "Bash:command|grep\nWrite:file_path",
                "2026-01-01",
                false,
            ),
            // A skill was distilled from one run of this shape → covered.
            shape(
                "b",
                "Bash:command|grep\nWrite:file_path",
                "2026-01-02",
                true,
            ),
        ];
        assert!(
            detect(shapes, 2).is_empty(),
            "a shape with an installed skill is not an opportunity"
        );
    }

    #[test]
    fn distinct_shapes_do_not_count_as_repetition() {
        let shapes = vec![
            shape("a", "Bash:command|grep", "2026-01-01", false),
            shape("b", "Read:file_path", "2026-01-02", false),
        ];
        // Two different one-offs, neither repeated → nothing at the default threshold.
        assert!(detect(shapes, 2).is_empty());
    }

    #[test]
    fn min_count_one_surfaces_single_undistilled_recordings() {
        let shapes = vec![shape(
            "a",
            "Bash:command|cargo\nRead:file_path",
            "2026-01-01",
            false,
        )];
        let opps = detect(shapes, 1);
        assert_eq!(opps.len(), 1);
        assert_eq!(opps[0].count, 1);
        assert!(opps[0].recommendation.contains("Recorded once"));
    }

    #[test]
    fn empty_signatures_are_ignored() {
        let shapes = vec![
            shape("a", "", "2026-01-01", false),
            shape("b", "", "2026-01-02", false),
        ];
        assert!(detect(shapes, 1).is_empty());
    }

    #[test]
    fn more_repeated_shapes_rank_above_less_repeated() {
        let shapes = vec![
            shape("a", "Bash:command|grep", "2026-01-01", false),
            shape("b", "Bash:command|grep", "2026-01-02", false),
            shape("c", "Bash:command|grep", "2026-01-03", false),
            shape("d", "Read:file_path\nWrite:file_path", "2026-01-04", false),
            shape("e", "Read:file_path\nWrite:file_path", "2026-01-05", false),
        ];
        let opps = detect(shapes, 2);
        assert_eq!(opps.len(), 2);
        // The 3× shape outranks the 2× shape.
        assert_eq!(opps[0].count, 3);
        assert_eq!(opps[1].count, 2);
        assert!(opps[0].score > opps[1].score);
    }
}
