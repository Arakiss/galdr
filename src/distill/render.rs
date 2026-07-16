/// Composes a complete `SKILL.md` from the span, in the open-standard anatomy
/// (`When to use` / `Inputs` / `Steps` / `Verification`) that Codex Record & Replay
/// also uses. Deterministic and LLM-free.
///
/// It renders clean by default, so the normal path clears the validation gate with no
/// hand-fixing: the description states what the task does (never the tautological
/// `use this when you need to <slug>`), recording scaffolding is dropped from the
/// steps, personal paths collapse to `~` (session/temp paths drop out entirely), and
/// the provenance keeps only the opaque `rec_id` plus timestamps.
fn render_complete_skill(
    skill_name: &str,
    recording: &record::Recording,
    events: &[Event],
) -> String {
    let home = paths::home_dir().map(|p| p.display().to_string());
    let home = home.as_deref();

    // Drop recording scaffolding (galdr control commands, sleeps, polling loops, bare
    // screenshots, throwaway temp reads) so the skill documents the task, not the
    // capture of it.
    let steps = meaningful_steps(events);
    let mut out = String::new();
    let tools = distinct_tools(&steps);
    let (kind, capability) = task_shape(&steps, &tools);
    let safe_name = one_line(&recording.name, 120);

    // Frontmatter. The description names what the task does and when to reach for it —
    // a real "when to use", not a restatement of the slug.
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "name: {skill_name}");
    let _ = writeln!(
        out,
        "description: \"Reproduce the recorded task \\\"{}\\\": a {}-step {} that {}. Use it to repeat this procedure with new inputs.\"",
        yaml_quoted(&recording.name),
        steps.len(),
        kind,
        capability
    );
    let _ = writeln!(out, "---");
    let _ = writeln!(out);
    let _ = writeln!(out, "# {skill_name}");
    let _ = writeln!(out);

    // When to use.
    let _ = writeln!(out, "## When to use");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Reach for this skill to reproduce **{}** — a {} that {}. It runs {} step{}; adapt the inputs below to the situation in front of you, then follow the steps with judgment (a guide to interpret, not a macro to replay verbatim).",
        safe_name,
        kind,
        capability,
        steps.len(),
        plural(steps.len())
    );
    let _ = writeln!(out);

    // Inputs — the recording's concrete values, offered as candidates.
    let _ = writeln!(out, "## Inputs");
    let _ = writeln!(out);
    let inputs = notable_inputs(&steps, home);
    if inputs.is_empty() {
        let _ = writeln!(
            out,
            "This task took no obvious varying inputs; the steps are self-contained. Record it twice with `galdr` and run `galdr parametrize` to extract real parameters."
        );
    } else {
        let _ = writeln!(
            out,
            "These values were specific to the recording. Replace them with the ones you need:"
        );
        for input in &inputs {
            let _ = writeln!(out, "- `{}` — {}", one_line(&input.value, 160), input.role);
        }
    }
    let _ = writeln!(out);

    // Steps.
    let _ = writeln!(out, "## Steps");
    let _ = writeln!(out);
    if steps.is_empty() {
        let _ = writeln!(out, "_(the recording captured no steps)_");
    } else {
        for (i, event) in steps.iter().enumerate() {
            // Redact secrets and generalize session-specific data (a typed password,
            // a personal path) so the step is reproducible and safe to share.
            let summary = crate::validate::generalize_session_text(
                &crate::export::redact_text(&summarize_event(event)),
                home,
            );
            let _ = writeln!(out, "{}. **{}** — {}", i + 1, step_title(event), summary);
        }
    }
    let _ = writeln!(out);

    render_known_forking_points(&mut out, recording, &steps, home);

    // Verification.
    let _ = writeln!(out, "## Verification");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{}",
        crate::validate::generalize_session_text(&verification_hint(&steps), home)
    );
    let _ = writeln!(out);

    // Provenance — the opaque rec_id (recoverable handle to the raw span) and the
    // timestamps. The cwd is reduced to its basename and the absolute span path is
    // dropped: both are personal paths, and the span is reachable via the rec_id.
    let _ = writeln!(out, "## Provenance");
    let _ = writeln!(out);
    let _ = writeln!(out, "- rec_id: `{}`", recording.rec_id);
    let _ = writeln!(
        out,
        "- recorded: {} → {}",
        recording.started_at, recording.ended_at
    );
    if let Some(base) = cwd_basename(recording.cwd.as_deref()) {
        let _ = writeln!(out, "- cwd (basename): `{base}`");
    }
    let _ = writeln!(out);

    out
}

fn render_known_forking_points(
    out: &mut String,
    recording: &record::Recording,
    steps: &[Event],
    home: Option<&str>,
) {
    let fork_points = crate::judge::summaries_for_distill(recording, steps);
    if fork_points.is_empty() {
        return;
    }
    let _ = writeln!(out, "## Known Forking Points");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "These are measured external judgments from on-policy attempts, not inferred guesses:"
    );
    for point in fork_points.iter().take(8) {
        let example = point.examples.first();
        let rationale = example
            .map(|example| safe_judgment_text(&example.rationale, home))
            .unwrap_or_else(|| "executor forked at this step".to_string());
        let action = example
            .and_then(|example| example.suggested_action.as_deref())
            .map(|action| safe_judgment_text(action, home));
        let _ = write!(
            out,
            "- Step {}: {}/{} judged attempt(s) forked. {}",
            point.step, point.forks, point.judged, rationale
        );
        if let Some(action) = action {
            let _ = write!(out, " Do this instead: {action}");
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out);
}

fn safe_judgment_text(text: &str, home: Option<&str>) -> String {
    one_line(
        &crate::validate::generalize_session_text(&crate::export::redact_text(text), home),
        240,
    )
}

/// Drops recording scaffolding from the steps, sharing the noise rubric with the
/// validation gate ([`crate::validate::is_noise_step`]). Guarded so the result is
/// never empty: a recording that is *only* scaffolding keeps its original steps
/// rather than distilling to a zero-step skill. Shared with `diff`, so two recordings
/// are aligned on the same meaningful steps a distilled skill would show.
pub(crate) fn meaningful_steps(events: &[Event]) -> Vec<Event> {
    let kept: Vec<Event> = events
        .iter()
        .filter(|e| {
            let summary = summarize_event(e);
            !crate::validate::is_noise_step(&e.tool_name, &summary)
        })
        .cloned()
        .collect();
    if kept.is_empty() {
        events.to_vec()
    } else {
        kept
    }
}

/// `""` for one item, `"s"` otherwise.
fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The basename of the recording's working directory, or `None`. The full path is a
/// personal path; the basename (the project folder) is the reusable signal.
fn cwd_basename(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd?;
    let base = std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .trim();
    (!base.is_empty() && base != "/").then(|| one_line(base, 80))
}

/// A short label and capability phrase for the task, derived from which tools it
/// used. Feeds the description and the "when to use" so both say what the task does
/// instead of restating its name.
fn task_shape(events: &[Event], tools: &[String]) -> (String, String) {
    let gui = tools.iter().any(|t| crate::summary::is_computer_use(t));
    let human_browser = tools.iter().any(|t| t.starts_with("human.browser."));
    let web = tools.iter().any(|t| {
        matches!(t.as_str(), "WebFetch" | "WebSearch")
            || t.contains("browser")
            || t.contains("playwright")
    });
    let file = tools.iter().any(|t| {
        matches!(
            t.as_str(),
            "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit"
        )
    });
    let bash = tools.iter().any(|t| t == "Bash");
    match (gui, human_browser, web, file, bash) {
        (true, _, _, _, _) => (
            "GUI workflow".to_string(),
            "drives a desktop application through Computer Use".to_string(),
        ),
        (_, true, _, _, _) => ("browser workflow".to_string(), browser_capability(events)),
        (_, _, true, _, _) => (
            "web task".to_string(),
            "fetches or searches the web".to_string(),
        ),
        (_, _, _, true, true) => (
            "task".to_string(),
            "edits files and runs shell commands".to_string(),
        ),
        (_, _, _, true, false) => (
            "file-editing task".to_string(),
            "reads and edits files".to_string(),
        ),
        (_, _, _, false, true) => (
            "command-line task".to_string(),
            "runs shell commands".to_string(),
        ),
        _ => (
            "multi-step task".to_string(),
            "drives a sequence of tools".to_string(),
        ),
    }
}

fn browser_capability(events: &[Event]) -> String {
    let has = |suffix: &str| {
        events.iter().any(|event| {
            event
                .human
                .as_ref()
                .map(|human| human.action.as_str().ends_with(suffix))
                .unwrap_or(false)
        })
    };
    match (
        has(".navigate"),
        has(".input"),
        has(".select"),
        has(".click"),
    ) {
        (true, true, true, true) => {
            "opens a page, fills labeled fields, chooses options, and activates the final control"
                .to_string()
        }
        (_, true, true, true) => {
            "fills labeled fields, chooses options, and activates the final control".to_string()
        }
        (_, true, _, true) => "fills browser fields and activates a control".to_string(),
        (true, _, _, true) => "opens a page and activates a browser control".to_string(),
        (true, _, _, _) => "opens the recorded browser page".to_string(),
        _ => "repeats the recorded browser interaction sequence".to_string(),
    }
}

fn step_title(event: &Event) -> String {
    let Some(human) = event.human.as_ref() else {
        return event.tool_name.clone();
    };
    let action = human
        .action
        .as_str()
        .strip_prefix("human.browser.")
        .or_else(|| human.action.as_str().strip_prefix("human."))
        .unwrap_or_else(|| human.action.as_str());
    match action {
        "navigate" => "Navigate",
        "input" => "Fill field",
        "select" => "Choose option",
        "click" => "Click",
        "check" => "Toggle option",
        "key" => "Press key",
        "submit" => "Submit",
        "wait" => "Wait",
        "download" => "Download",
        _ => "Human action",
    }
    .to_string()
}

/// The distinct tool names in the recording, in first-seen order.
fn distinct_tools(events: &[Event]) -> Vec<String> {
    let mut seen = Vec::new();
    for event in events {
        if !seen.contains(&event.tool_name) {
            seen.push(event.tool_name.clone());
        }
    }
    seen
}

/// One concrete value the recording used, with the role it played.
struct NotableInput {
    value: String,
    role: String,
}

/// Pulls the recording's notable concrete values — file paths, URLs, queries — as
/// candidate inputs. Step numbers match the rendered (filtered) steps. A secret is
/// redacted and a personal path is generalized to `~`; a session/temp path is dropped
/// outright, since `<temp path>` is no use as a reusable input.
fn notable_inputs(events: &[Event], home: Option<&str>) -> Vec<NotableInput> {
    let mut inputs: Vec<NotableInput> = Vec::new();
    let field = |event: &Event, key: &str| {
        event
            .tool_input
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    for (i, event) in events.iter().enumerate() {
        let step = i + 1;
        let candidate =
            human_notable_input(event, step).or_else(|| match event.tool_name.as_str() {
                "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
                    field(event, "file_path").map(|value| NotableInput {
                        value,
                        role: format!("file at step {step} ({})", event.tool_name),
                    })
                }
                "WebFetch" | "WebSearch" => field(event, "url")
                    .or_else(|| field(event, "query"))
                    .map(|value| NotableInput {
                        value,
                        role: format!("web target at step {step}"),
                    }),
                // Computer Use: the text *typed* into the GUI is what varies between runs
                // — surface it as a candidate input, not coordinates or keystrokes.
                name if crate::summary::is_computer_use(name)
                    && field(event, "action").as_deref() == Some("type") =>
                {
                    field(event, "text")
                        .filter(|t| !t.trim().is_empty())
                        .map(|value| NotableInput {
                            value,
                            role: format!("text typed at step {step}"),
                        })
                }
                _ => None,
            });
        if let Some(input) = candidate {
            let NotableInput { value, role } = input;
            if value.is_empty() || crate::validate::is_temp_path(&value) {
                continue;
            }
            let value =
                crate::validate::generalize_session_text(&crate::export::redact_text(&value), home);
            if !value.is_empty() && !inputs.iter().any(|i| i.value == value) {
                inputs.push(NotableInput { value, role });
            }
        }
    }
    inputs.truncate(12);
    inputs
}

fn human_notable_input(event: &Event, step: usize) -> Option<NotableInput> {
    let human = event.human.as_ref()?;
    let action = human
        .action
        .as_str()
        .strip_prefix("human.browser.")
        .unwrap_or_else(|| human.action.as_str());
    match (action, human.value.as_ref()?) {
        ("input", HumanValue::Literal { value }) if !value.trim().is_empty() => {
            let target = human_target_label(human).unwrap_or_else(|| "text value".to_string());
            Some(NotableInput {
                value: format!("<{target}>"),
                role: format!(
                    "text for {target} at step {step} ({} chars recorded)",
                    value.chars().count()
                ),
            })
        }
        ("input", HumanValue::Redacted { kind, chars }) => {
            let target = human_target_label(human).unwrap_or_else(|| "text value".to_string());
            Some(NotableInput {
                value: format!("<{target}>"),
                role: chars
                    .map(|chars| {
                        format!(
                            "redacted {kind} for {target} at step {step} ({chars} chars recorded)"
                        )
                    })
                    .unwrap_or_else(|| format!("redacted {kind} for {target} at step {step}")),
            })
        }
        ("input", HumanValue::Omitted { reason }) => {
            let target = human_target_label(human).unwrap_or_else(|| "omitted value".to_string());
            Some(NotableInput {
                value: format!("<{target}>"),
                role: format!("omitted value for {target} at step {step} ({reason})"),
            })
        }
        ("select", HumanValue::Literal { value }) if !value.trim().is_empty() => {
            let target = human_target_label(human)
                .map(|target| format!(" for {target}"))
                .unwrap_or_default();
            Some(NotableInput {
                value: value.clone(),
                role: format!("selection{target} at step {step}"),
            })
        }
        ("key", HumanValue::Literal { value }) if !value.trim().is_empty() => Some(NotableInput {
            value: value.clone(),
            role: format!("key pressed at step {step}"),
        }),
        _ => None,
    }
}

fn human_target_label(human: &HumanEvent) -> Option<String> {
    let target = human.target.as_ref()?;
    for value in [
        target.label.as_deref(),
        target.name.as_deref(),
        target.text.as_deref(),
        target.placeholder.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !value.trim().is_empty() {
            return Some(one_line(value, 80));
        }
    }
    match &target.primary {
        TargetLocator::Role {
            name: Some(name), ..
        }
        | TargetLocator::Label { value: name }
        | TargetLocator::Placeholder { value: name }
        | TargetLocator::TestId { value: name } => Some(one_line(name, 80)),
        TargetLocator::Role { name: None, role } => Some(one_line(role, 80)),
        TargetLocator::Css { .. } | TargetLocator::XPath { .. } => None,
    }
}

/// A verification line derived from the recording's last meaningful step.
fn verification_hint(events: &[Event]) -> String {
    let Some(last) = events.last() else {
        return "Confirm the task completed as intended; the recording captured no steps to check."
            .to_string();
    };
    if let Some(hint) = last
        .human
        .as_ref()
        .and_then(|human| human.verification_hint.as_deref())
    {
        return hint.to_string();
    }
    if let Some(human) = last.human.as_ref() {
        let action = human
            .action
            .as_str()
            .strip_prefix("human.browser.")
            .unwrap_or_else(|| human.action.as_str());
        match action {
            "navigate" => return "Confirm the expected browser page is open.".to_string(),
            "click" | "submit" => {
                if let Some(target) = human_target_label(human) {
                    return format!(
                        "Confirm the page shows the expected result after activating {target}."
                    );
                }
                return "Confirm the page shows the expected result after the final browser action."
                    .to_string();
            }
            "input" | "select" | "check" => {
                return "Confirm the browser form shows the intended values before continuing."
                    .to_string();
            }
            _ => {}
        }
    }
    match last.tool_name.as_str() {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => last
            .tool_input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(|p| format!("Confirm `{p}` exists and contains the intended changes."))
            .unwrap_or_else(|| "Confirm the edited file holds the intended changes.".to_string()),
        "Bash" => {
            "Confirm the commands ran without error (exit 0) and produced the expected output."
                .to_string()
        }
        _ => "Confirm the final step produced the intended result, and that each prior step succeeded.".to_string(),
    }
}
