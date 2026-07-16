// ----------------------------------------------------------------------------
// Practicality
// ----------------------------------------------------------------------------

fn practicality_findings(md: &str, report: &mut ValidationReport) {
    // The structural contract galdr already enforces for `--from` installs.
    if let Err(err) = crate::distill::validate_skill_md(md) {
        report.push(
            Severity::Error,
            Category::Practicality,
            "PRAC_STRUCTURE",
            format!("{err:#}"),
            None,
        );
    }
    for section in empty_required_sections(md) {
        report.push(
            Severity::Error,
            Category::Practicality,
            "PRAC_EMPTY_SECTION",
            format!("required section `## {section}` is empty"),
            None,
        );
    }
}

/// Required sections present as a heading but with no content before the next
/// heading. A section absent entirely is reported by the structural check, not here.
fn empty_required_sections(md: &str) -> Vec<&'static str> {
    const REQUIRED: &[&str] = &[
        "When to use",
        "Steps",
        "Verification",
        "Goal",
        "Procedure",
        "Success criteria",
    ];
    let mut empty = Vec::new();
    for &section in REQUIRED {
        if let Some(body) = section_body(md, section)
            && body.trim().is_empty()
        {
            empty.push(section);
        }
    }
    empty
}

/// The text of a `## <section>` heading up to the next `## ` heading, if present.
fn section_body<'a>(md: &'a str, section: &str) -> Option<&'a str> {
    let heading = format!("## {section}");
    let mut start_byte = None;
    let mut offset = 0;
    for line in md.lines() {
        if line.trim().eq_ignore_ascii_case(&heading) {
            start_byte = Some(offset + line.len());
            break;
        }
        offset += line.len() + 1;
    }
    let rest = &md[start_byte?.min(md.len())..];
    // Cut at the next level-2 heading.
    let end = rest
        .match_indices("\n## ")
        .next()
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    Some(&rest[..end])
}
