//! Harness setup helpers.

use anyhow::Result;

use crate::paths;

const CLAUDE_HOOK_SNIPPET: &str = r#"{
  "hooks": {
    "PostToolUse": [
      {
        "hooks": [
          { "type": "command", "command": "galdr hook" }
        ]
      }
    ]
  }
}"#;

pub fn claude_check() -> Result<()> {
    let path = paths::claude_settings()?;
    let Ok(contents) = std::fs::read_to_string(&path) else {
        println!("Claude Code settings not found: {}", path.display());
        println!("Run `galdr setup claude --print` to see the hook snippet.");
        return Ok(());
    };
    let configured = serde_json::from_str::<serde_json::Value>(&contents)
        .map(|value| has_galdr_hook(&value))
        .unwrap_or_else(|_| contents.contains("PostToolUse") && contents.contains("galdr hook"));
    if configured {
        println!(
            "Claude Code PostToolUse hook is configured: {}",
            path.display()
        );
    } else {
        println!(
            "Claude Code PostToolUse hook is missing: {}",
            path.display()
        );
        println!("Run `galdr setup claude --print` and merge the snippet into settings.json.");
    }
    Ok(())
}

pub fn claude_print() {
    println!("{CLAUDE_HOOK_SNIPPET}");
}

pub fn claude_hook_configured() -> Option<bool> {
    let path = paths::claude_settings().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&contents)
        .map(|value| has_galdr_hook(&value))
        .ok()
}

pub fn codex_check() -> Result<()> {
    let path = paths::codex_hooks()?;
    let Ok(contents) = std::fs::read_to_string(&path) else {
        println!("Codex hooks file not found: {}", path.display());
        println!("Run `galdr setup codex --print` to see the hook snippet.");
        return Ok(());
    };
    let configured = serde_json::from_str::<serde_json::Value>(&contents)
        .map(|value| has_galdr_hook(&value))
        .unwrap_or_else(|_| contents.contains("PostToolUse") && contents.contains("galdr hook"));
    if configured {
        println!("Codex PostToolUse hook is configured: {}", path.display());
    } else {
        println!("Codex PostToolUse hook is missing: {}", path.display());
        println!("Run `galdr setup codex --print` and merge the snippet into hooks.json.");
    }
    Ok(())
}

pub fn codex_print() {
    // Codex's hooks.json shares Claude Code's PostToolUse shape, so the same snippet
    // applies — merge it into ~/.codex/hooks.json (hook arrays are concatenated).
    println!("{CLAUDE_HOOK_SNIPPET}");
}

pub fn codex_hook_configured() -> Option<bool> {
    let path = paths::codex_hooks().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&contents)
        .map(|value| has_galdr_hook(&value))
        .ok()
}

fn has_galdr_hook(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            if map
                .get("command")
                .and_then(|v| v.as_str())
                .is_some_and(is_galdr_hook_command)
            {
                return true;
            }
            map.values().any(has_galdr_hook)
        }
        serde_json::Value::Array(values) => values.iter().any(has_galdr_hook),
        _ => false,
    }
}

/// True if `command` invokes `galdr hook`. Accepts the bare `galdr hook`, a
/// path-qualified `/path/to/galdr hook`, and — crucially — `galdr hook` embedded
/// in a shell wrapper such as
/// `if command -v galdr …; then galdr hook; elif …; then "$HOME/.cargo/bin/galdr" hook; fi`,
/// which is the resilient form many users wire up. It must not be fooled by a
/// neighbouring `galdr hooks` or `galdr outcome`.
fn is_galdr_hook_command(command: &str) -> bool {
    // Split on whitespace and shell separators, strip surrounding quotes, then look
    // for a program token ending in `galdr` immediately followed by the `hook` arg.
    let tokens: Vec<String> = command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
        .map(|t| t.trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|t| !t.is_empty())
        .collect();
    tokens.windows(2).any(|pair| {
        let prog = pair[0].as_str();
        (prog == "galdr" || prog.ends_with("/galdr")) && pair[1] == "hook"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_path_qualified_galdr_hook_commands() {
        assert!(is_galdr_hook_command("galdr hook"));
        assert!(is_galdr_hook_command(
            "/Users/dolores/.cargo/bin/galdr hook"
        ));
        assert!(!is_galdr_hook_command("galdr outcome list"));
    }

    #[test]
    fn recognizes_galdr_hook_wrapped_in_a_shell_conditional() {
        // The resilient form people actually wire up: a shell `if/elif` that finds
        // galdr on PATH or falls back to the cargo bin. Both branches invoke it.
        let wrapped = r#"if command -v galdr >/dev/null 2>&1; then galdr hook; elif [ -x "$HOME/.cargo/bin/galdr" ]; then "$HOME/.cargo/bin/galdr" hook; fi"#;
        assert!(is_galdr_hook_command(wrapped));
    }

    #[test]
    fn does_not_match_other_galdr_subcommands_or_lookalikes() {
        assert!(!is_galdr_hook_command("galdr hooks"));
        assert!(!is_galdr_hook_command("command -v galdr"));
        assert!(!is_galdr_hook_command("echo galdr && run hook"));
    }
}
