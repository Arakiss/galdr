//! Shared, side-effect-free helpers for summarizing and naming.
//!
//! These started life in `distill.rs`; the catalog, the TUI, the diff, and the
//! parametrizer all need the exact same one-line summary of a tool call and the
//! same slug rules. Keeping a single source of truth here means a span step reads
//! identically wherever it is shown.

/// Turns a name into a slug suitable for a skill directory.
pub(crate) fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "rec".to_string()
    } else {
        slug
    }
}

/// Truncates text to `max` characters, collapsing whitespace and adding an
/// ellipsis if it is cut.
pub(crate) fn truncate(text: &str, max: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let head: String = one_line.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Drops leading `cd <dir> &&` / `cd <dir>;` / `cd <dir>\n` segments from a shell
/// command so the summary shows the meaningful command, not the boilerplate the
/// harness prepends. A bare `cd <dir>` with nothing after it is kept as-is.
fn strip_leading_cd(command: &str) -> String {
    let mut cmd = command.trim();
    while let Some(after_cd) = cmd.strip_prefix("cd ") {
        let nl = after_cd.find('\n');
        let semi = after_cd.find(';');
        let amp = after_cd.find("&&");
        let Some(pos) = [nl, semi, amp].into_iter().flatten().min() else {
            break; // just `cd <dir>` — nothing meaningful follows, keep it
        };
        let sep_len = if after_cd[pos..].starts_with("&&") {
            2
        } else {
            1
        };
        let next = after_cd[pos + sep_len..].trim_start();
        if next.is_empty() {
            break;
        }
        cmd = next;
    }
    cmd.to_string()
}

/// Summarizes a tool call's input on one line, according to the tool. This is the
/// summary stored in the catalog and shown in every list: never the raw blob.
pub(crate) fn summarize_input(tool_name: &str, input: &serde_json::Value) -> String {
    let field = |key: &str| input.get(key).and_then(|v| v.as_str()).map(str::to_string);

    let raw = match tool_name {
        "Bash" => field("command").map(|c| strip_leading_cd(&c)),
        "Read" | "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => field("file_path"),
        "Glob" => field("pattern"),
        "Grep" => field("pattern").map(|p| {
            field("path")
                .map(|path| format!("{p}  in {path}"))
                .unwrap_or(p)
        }),
        "WebFetch" | "WebSearch" => field("url").or_else(|| field("query")),
        name if is_computer_use(name) => Some(describe_computer_use(name, input)),
        _ => None,
    };

    let raw = raw.unwrap_or_else(|| describe_unknown(input));

    truncate(&raw, 160)
}

/// True for Claude's Computer Use tool (the built-in `computer-use` MCP server) and
/// the classic `computer` tool. Matched loosely so a renamed MCP variant still hits.
pub(crate) fn is_computer_use(tool_name: &str) -> bool {
    let t = tool_name.to_ascii_lowercase();
    t == "computer" || t.contains("computer_use") || t.contains("computer-use")
}

/// Renders a Computer Use action on one line — `left_click (812,344)`, `type "42.50"`,
/// `key "cmd+s"`, `screenshot`, `open_application "Calculator"` — so a recorded GUI
/// session reads like the steps the agent took, not a wall of coordinates and base64.
/// The pixels themselves are never the reusable signal; the action is.
///
/// Two shapes exist in the wild and both are handled:
/// - the **classic single tool** `computer`, where the verb is an `action` field
///   (`{action:"left_click", coordinate:[x,y]}`);
/// - the **per-action MCP server** (`mcp__computer-use__left_click`,
///   `…__screenshot`, `…__open_application`, `…__computer_batch`), where the verb is
///   the tool-name suffix and a `computer_batch` carries an `actions` array.
fn describe_computer_use(tool_name: &str, input: &serde_json::Value) -> String {
    let serde_json::Value::Object(map) = input else {
        return action_verb_from_name(tool_name).to_string();
    };
    // The classic tool names the verb in an `action` field; the per-action server
    // names it in the tool itself (the suffix after the last `__`).
    let verb = map
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| action_verb_from_name(tool_name));

    // A batch is a sequence of sub-actions; render the sequence so the GUI skill
    // reads as the steps taken, not an opaque "batch".
    if let Some(actions) = map.get("actions").and_then(|v| v.as_array()) {
        let parts: Vec<String> = actions
            .iter()
            .filter_map(|a| {
                let m = a.as_object()?;
                let sub = m.get("action").and_then(|v| v.as_str()).unwrap_or("action");
                Some(render_action(sub, m))
            })
            .collect();
        if !parts.is_empty() {
            return format!("{verb} ×{}: {}", parts.len(), parts.join(", "));
        }
    }

    render_action(verb, map)
}

/// The action verb encoded in a Computer Use tool name: the suffix after the last
/// `__` (`mcp__computer-use__left_click` → `left_click`), or the whole name for a
/// bare tool (`computer` → `computer`).
fn action_verb_from_name(tool_name: &str) -> &str {
    tool_name.rsplit("__").next().unwrap_or(tool_name)
}

/// Renders one Computer Use action (`verb (x,y)` / `verb "text"` / `verb "app"` /
/// `verb`) from its verb and the object carrying its parameters. Shared by the
/// top-level call and each sub-action of a `computer_batch`.
fn render_action(verb: &str, map: &serde_json::Map<String, serde_json::Value>) -> String {
    let str_of = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
    };

    // Coordinate-bearing actions: clicks, moves, drags, scroll.
    if let Some(coord) = coordinate_str(map) {
        if let Some(dir) = str_of(&["scroll_direction"]) {
            return format!("{verb} {dir} {coord}");
        }
        return format!("{verb} {coord}");
    }
    // Text/key-bearing actions: type, key, hold_key.
    if let Some(text) = str_of(&["text", "key"]) {
        return format!("{verb} \"{text}\"");
    }
    // App-bearing actions: open_application, switch_display.
    if let Some(app) = str_of(&["app", "application", "bundleId", "name"]) {
        return format!("{verb} \"{app}\"");
    }
    // request_access takes an `apps` array of names.
    if let Some(apps) = map.get("apps").and_then(|v| v.as_array()) {
        let names: Vec<&str> = apps.iter().filter_map(|v| v.as_str()).collect();
        if !names.is_empty() {
            return format!("{verb} \"{}\"", names.join(", "));
        }
    }
    verb.to_string()
}

/// Formats a `coordinate` value as `(x,y)`. Accepts both `[x, y]` and `{x, y}`.
fn coordinate_str(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    match map.get("coordinate")? {
        serde_json::Value::Array(a) if a.len() == 2 => Some(format!("({},{})", a[0], a[1])),
        serde_json::Value::Object(o) => match (o.get("x"), o.get("y")) {
            (Some(x), Some(y)) => Some(format!("({x},{y})")),
            _ => None,
        },
        _ => None,
    }
}

/// Summarizes a tool call galdr has no special case for — most importantly the MCP
/// and browser tools an agent drives (`mcp__playwright__browser_click`, …). Their
/// web actions are already captured as plain tool calls; this just renders the
/// informative value (url, selector, text…) instead of a bare list of field names,
/// so a recorded browser session reads like steps, not like JSON keys.
fn describe_unknown(input: &serde_json::Value) -> String {
    let serde_json::Value::Object(map) = input else {
        return match input {
            serde_json::Value::Null => "(no input)".to_string(),
            other => other.to_string(),
        };
    };
    // The fields most likely to carry the meaning, in priority order.
    const INFORMATIVE: &[&str] = &[
        "url",
        "selector",
        "text",
        "query",
        "path",
        "file_path",
        "command",
        "name",
        "message",
        "body",
        "content",
        "pattern",
        "value",
        "key",
    ];
    let mut shown: Vec<String> = Vec::new();
    for key in INFORMATIVE {
        if let Some(value) = map.get(*key).and_then(|v| v.as_str())
            && !value.trim().is_empty()
        {
            shown.push(format!("{key}={value}"));
            if shown.len() == 2 {
                break;
            }
        }
    }
    if shown.is_empty() {
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        format!("fields: {}", keys.join(", "))
    } else {
        // A middle dot survives `truncate`'s whitespace collapse, unlike a run of
        // spaces, so the two values stay visually separated.
        shown.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::{is_computer_use, slugify, summarize_input, truncate};

    #[test]
    fn slugify_normalizes_names() {
        assert_eq!(slugify("Git Change Summary"), "git-change-summary");
        assert_eq!(slugify("  weird__name!! "), "weird-name");
        assert_eq!(slugify("!!!"), "rec");
    }

    #[test]
    fn truncate_collapses_and_caps() {
        assert_eq!(truncate("a b  c", 80), "a b c");
        assert!(truncate(&"x".repeat(200), 10).ends_with('…'));
    }

    #[test]
    fn summarize_strips_leading_cd_boilerplate() {
        assert_eq!(
            summarize_input(
                "Bash",
                &serde_json::json!({ "command": "cd /a/b/c\ngit log --oneline" })
            ),
            "git log --oneline"
        );
        assert_eq!(
            summarize_input(
                "Bash",
                &serde_json::json!({ "command": "cd /x && cd /y && cargo test" })
            ),
            "cargo test"
        );
        // A bare cd is meaningful on its own — keep it.
        assert_eq!(
            summarize_input("Bash", &serde_json::json!({ "command": "cd /only" })),
            "cd /only"
        );
    }

    #[test]
    fn summarize_reads_tool_specific_fields() {
        assert_eq!(
            summarize_input("Bash", &serde_json::json!({ "command": "git status" })),
            "git status"
        );
        assert_eq!(
            summarize_input("Write", &serde_json::json!({ "file_path": "/tmp/x.md" })),
            "/tmp/x.md"
        );
        assert_eq!(
            summarize_input("Unknown", &serde_json::json!({ "a": 1, "b": 2 })),
            "fields: a, b"
        );
    }

    #[test]
    fn summarize_renders_computer_use_actions() {
        assert!(is_computer_use("mcp__computer-use__computer"));
        assert!(is_computer_use("computer"));
        assert!(!is_computer_use("Bash"));
        // Classic single `computer` tool: the verb is in an `action` field.
        assert_eq!(
            summarize_input(
                "mcp__computer-use__computer",
                &serde_json::json!({ "action": "left_click", "coordinate": [812, 344] })
            ),
            "left_click (812,344)"
        );
        assert_eq!(
            summarize_input(
                "mcp__computer-use__computer",
                &serde_json::json!({ "action": "type", "text": "42.50" })
            ),
            "type \"42.50\""
        );
        assert_eq!(
            summarize_input("computer", &serde_json::json!({ "action": "screenshot" })),
            "screenshot"
        );
    }

    #[test]
    fn summarize_renders_per_action_computer_use_server() {
        // The real `computer-use` MCP server uses one tool per action: the verb is
        // the tool-name suffix, and the parameters sit at the top level.
        assert_eq!(
            summarize_input("mcp__computer-use__screenshot", &serde_json::json!({})),
            "screenshot"
        );
        assert_eq!(
            summarize_input(
                "mcp__computer-use__left_click",
                &serde_json::json!({ "coordinate": [398, 339] })
            ),
            "left_click (398,339)"
        );
        assert_eq!(
            summarize_input(
                "mcp__computer-use__open_application",
                &serde_json::json!({ "app": "Calculadora" })
            ),
            "open_application \"Calculadora\""
        );
        assert_eq!(
            summarize_input(
                "mcp__computer-use__request_access",
                &serde_json::json!({ "apps": ["Calculadora"], "reason": "demo" })
            ),
            "request_access \"Calculadora\""
        );
        assert_eq!(
            summarize_input(
                "mcp__computer-use__type",
                &serde_json::json!({ "text": "42" })
            ),
            "type \"42\""
        );
    }

    #[test]
    fn summarize_renders_a_computer_batch_as_its_sequence() {
        let summary = summarize_input(
            "mcp__computer-use__computer_batch",
            &serde_json::json!({ "actions": [
                { "action": "left_click", "coordinate": [398, 339] },
                { "action": "left_click", "coordinate": [372, 388] },
            ] }),
        );
        assert!(summary.starts_with("computer_batch ×2:"), "{summary}");
        assert!(summary.contains("left_click (398,339)"), "{summary}");
        assert!(summary.contains("left_click (372,388)"), "{summary}");
    }

    #[test]
    fn summarize_renders_browser_and_mcp_tool_values() {
        // An agent's browser tool calls are captured as plain tool calls; the
        // summary should show the informative value, not just the field names.
        assert_eq!(
            summarize_input(
                "mcp__playwright__browser_navigate",
                &serde_json::json!({ "url": "https://app.example.com/expenses" })
            ),
            "url=https://app.example.com/expenses"
        );
        assert_eq!(
            summarize_input(
                "mcp__playwright__browser_type",
                &serde_json::json!({ "selector": "#amount", "text": "42.50" })
            ),
            "selector=#amount · text=42.50"
        );
    }
}
