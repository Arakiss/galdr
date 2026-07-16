/// Scans the skills directory for `(skill_name, rec_id, skill_path)`. Tolerant:
/// an unreadable or unparseable skill is skipped, and a missing provenance line
/// just yields `None` for the rec_id (which the catalog flags as an orphan).
struct SkillScan {
    name: String,
    rec_id: Option<String>,
    path: String,
    status: String,
}

fn scan_skills_raw(skills_root: &Path) -> Vec<SkillScan> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(skills_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let skill_md = dir.join("SKILL.md");
        let Ok(contents) = std::fs::read_to_string(&skill_md) else {
            continue;
        };
        let name = parse_frontmatter_name(&contents)
            .or_else(|| dir.file_name().and_then(|s| s.to_str()).map(str::to_string))
            .unwrap_or_else(|| "skill".to_string());
        let rec_id = parse_rec_id(&contents);
        let status = infer_skill_status(&contents);
        out.push(SkillScan {
            name,
            rec_id,
            path: skill_md.to_string_lossy().into_owned(),
            status,
        });
    }
    out
}

struct SkillReadiness {
    score: i64,
    notes: String,
    confidence: f64,
    rationale: serde_json::Value,
}

fn analyze_skill_file(skill_path: &str, rec_id: Option<&str>) -> SkillReadiness {
    let Ok(contents) = std::fs::read_to_string(skill_path) else {
        return SkillReadiness {
            score: 0,
            notes: "skill file unreadable".to_string(),
            confidence: 0.2,
            rationale: serde_json::json!({
                "metric": "readiness_lint_v2",
                "error": "skill file unreadable"
            }),
        };
    };
    analyze_skill_md(&contents, rec_id)
}

fn analyze_skill_md(md: &str, rec_id: Option<&str>) -> SkillReadiness {
    let mut score = 100_i64;
    let mut notes = Vec::new();

    let has_name = has_frontmatter_name(md);
    let has_description = has_frontmatter_description(md);
    if !has_name {
        score -= 20;
        notes.push("missing frontmatter name");
    }
    if !has_description {
        score -= 20;
        notes.push("missing frontmatter description");
    }
    let missing_sections = missing_procedural_sections(md);
    if !missing_sections.is_empty() {
        score -= 10 * missing_sections.len() as i64;
        notes.push("missing required sections");
    }
    let draft_markers_present = md.contains("TODO(agent)")
        || md.contains("[galdr DRAFT]")
        || md.contains("galdr:unauthored");
    if draft_markers_present {
        score -= 25;
        notes.push("draft markers present");
    }
    let provenance_present = rec_id.is_some();
    if !provenance_present {
        score -= 10;
        notes.push("missing provenance");
    }
    // Optimization signals, sharing the gate's rubric (`readiness_lint_v2`): a
    // description that just restates the name, and steps that are recording noise
    // rather than the task itself. Both warn; neither is fatal to readiness.
    let tautological_description = crate::validate::is_tautological_description(md);
    if tautological_description {
        score -= 10;
        notes.push("tautological description");
    }
    let noise_steps = crate::validate::noise_step_count(md);
    if noise_steps > 0 {
        score -= 5;
        notes.push("recording-noise steps");
    }

    SkillReadiness {
        score: score.max(0),
        notes: if notes.is_empty() {
            "ready".to_string()
        } else {
            notes.join("; ")
        },
        confidence: 0.95,
        rationale: serde_json::json!({
            "metric": "readiness_lint_v2",
            "frontmatter": {
                "name": has_name,
                "description": has_description
            },
            "required_sections": {
                "missing": missing_sections
            },
            "draft_markers_present": draft_markers_present,
            "provenance_present": provenance_present,
            "tautological_description": tautological_description,
            "noise_steps": noise_steps
        }),
    }
}

pub fn infer_skill_status(md: &str) -> String {
    if md.contains("Parametrized from two recordings") || md.contains("## Procedure (parametrized)")
    {
        return STATUS_PARAM_DRAFT.to_string();
    }
    if md.contains("TODO(agent)") || md.contains("[galdr DRAFT]") || md.contains("galdr:unauthored")
    {
        return STATUS_DRAFT.to_string();
    }
    if missing_procedural_sections(md).is_empty() {
        return STATUS_FINAL.to_string();
    }
    STATUS_UNKNOWN.to_string()
}

/// The procedural sections a complete skill still lacks. A skill is complete under
/// either anatomy: the open-standard / Codex shape galdr now emits
/// (`When to use` / `Steps` / `Verification`) or the legacy trio
/// (`Goal` / `Procedure` / `Success criteria`). A fully-present legacy skill counts
/// as complete; otherwise the Codex sections are the ones reported missing.
fn missing_procedural_sections(md: &str) -> Vec<&'static str> {
    const CODEX: [&str; 3] = ["When to use", "Steps", "Verification"];
    const LEGACY: [&str; 3] = ["Goal", "Procedure", "Success criteria"];
    if LEGACY.iter().all(|s| has_section(md, s)) {
        return Vec::new();
    }
    CODEX
        .iter()
        .copied()
        .filter(|s| !has_section(md, s))
        .collect()
}

fn has_frontmatter_name(md: &str) -> bool {
    parse_frontmatter_name(md).is_some()
}

fn has_frontmatter_description(md: &str) -> bool {
    let mut lines = md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.starts_with("description:") {
            return true;
        }
    }
    false
}

fn has_section(md: &str, section: &str) -> bool {
    md.lines()
        .any(|line| line.trim().eq_ignore_ascii_case(&format!("## {section}")))
}

pub(crate) fn parse_frontmatter_name(md: &str) -> Option<String> {
    let mut lines = md.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

pub(crate) fn parse_rec_id(md: &str) -> Option<String> {
    for line in md.lines() {
        if let Some(idx) = line.find("rec_id:") {
            let after = &line[idx + "rec_id:".len()..];
            let token = after
                .trim()
                .trim_matches('`')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('`');
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}
