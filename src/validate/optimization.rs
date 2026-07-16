// ----------------------------------------------------------------------------
// Optimization
// ----------------------------------------------------------------------------

fn optimization_findings(md: &str, report: &mut ValidationReport) {
    if is_tautological_description(md) {
        report.push(
            Severity::Warn,
            Category::Optimization,
            "OPT_TAUTOLOGICAL_DESC",
            "the description's \"when to use\" just restates the task name; say what the task does and when to reach for it",
            None,
        );
    }
    for (lineno, summary) in noise_step_lines(md) {
        report.push(
            Severity::Warn,
            Category::Optimization,
            "OPT_NOISE_STEP",
            format!("step looks like recording noise, not part of the task: {summary}"),
            Some(lineno),
        );
    }
    const MAX_CHARS: usize = 16_000;
    if md.len() > MAX_CHARS {
        report.push(
            Severity::Warn,
            Category::Optimization,
            "OPT_LONG",
            format!(
                "skill is {} chars (> {MAX_CHARS}); tighten it so an agent can hold it",
                md.len()
            ),
            None,
        );
    }
}

/// True when the frontmatter description's guidance adds nothing beyond the skill
/// name — the classic broken render `Use this when you need to <slug>`.
pub(crate) fn is_tautological_description(md: &str) -> bool {
    let Some(desc) = frontmatter_value(md, "description") else {
        return false;
    };
    let desc = desc.to_ascii_lowercase();
    let Some(marker_at) = desc.find("you need to ") else {
        return false;
    };
    let clause = &desc[marker_at + "you need to ".len()..];
    let clause = clause.split('.').next().unwrap_or(clause);
    let clause_slug = slugify(clause);
    if clause_slug.is_empty() {
        return false;
    }
    let name_slug = frontmatter_value(md, "name")
        .map(|n| slugify(n.trim_start_matches("galdr-")))
        .unwrap_or_default();
    !name_slug.is_empty() && (clause_slug == name_slug || name_slug.contains(&clause_slug))
}

/// How many rendered step lines look like recording noise. Shared with the catalog's
/// readiness lint so the gate and the score judge a skill by the same rubric.
pub(crate) fn noise_step_count(md: &str) -> usize {
    noise_step_lines(md).len()
}

/// The numbered `## Steps` lines that look like recording noise rather than task
/// work, as `(line_number, summary)`.
fn noise_step_lines(md: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (idx, line) in md.lines().enumerate() {
        if let Some((tool, summary)) = parse_step_line(line)
            && is_noise_step(tool, summary)
        {
            out.push((idx + 1, summary.to_string()));
        }
    }
    out
}

/// Parses a rendered step line `N. **Tool** — summary` into `(tool, summary)`.
fn parse_step_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let rest = line.split_once(". **")?.1;
    let (tool, after) = rest.split_once("** — ")?;
    Some((tool, after))
}

/// Whether a step is recording scaffolding (galdr control commands, a sleep, a
/// polling loop, a bare screenshot, or reading a throwaway temp file) rather than a
/// meaningful part of the task. Shared by the distiller's filter and the gate so the
/// two never disagree.
pub(crate) fn is_noise_step(tool: &str, summary: &str) -> bool {
    let s = summary.trim();
    if s.contains("galdr rec start")
        || s.contains("galdr rec stop")
        || s.contains("galdr rec status")
    {
        return true;
    }
    // A bare sleep, or a polling loop built around one.
    if is_bare_sleep(s) || (s.starts_with("while ") && s.contains("sleep")) {
        return true;
    }
    // A standalone Computer Use screenshot — the pixels are never the reusable signal.
    if s == "screenshot" {
        return true;
    }
    // Reading or cat-ing a throwaway temp file (but a real file is kept).
    if (tool == "Read" && is_temp_path(s)) || (tool == "Bash" && is_temp_cat(s)) {
        return true;
    }
    false
}

fn is_bare_sleep(s: &str) -> bool {
    s.strip_prefix("sleep ")
        .map(|rest| {
            let head = rest.split_whitespace().next().unwrap_or("");
            !head.is_empty() && head.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .unwrap_or(false)
}

fn is_temp_cat(s: &str) -> bool {
    s.strip_prefix("cat ")
        .map(|rest| rest.split_whitespace().next().is_some_and(is_temp_path))
        .unwrap_or(false)
}

pub(crate) fn is_temp_path(s: &str) -> bool {
    let s = s.trim_matches(|c| matches!(c, '`' | '"' | '\''));
    s.starts_with("/tmp/")
        || s.starts_with("/private/tmp/")
        || s.starts_with("/var/folders/")
        || s.contains("/scratchpad/")
}
