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
        _ => None,
    };

    let raw = raw.unwrap_or_else(|| describe_unknown(input));

    truncate(&raw, 160)
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
    use super::{slugify, summarize_input, truncate};

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
