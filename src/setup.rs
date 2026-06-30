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

// Codex's hooks system is native and modeled on Claude Code's: same PostToolUse event
// and stdin payload (`tool_name`/`tool_input`/`tool_response`/…), plus a `matcher` regex
// and per-hook `timeout`. The one thing Codex adds — and the reason a merged hook can
// silently never fire — is a trust gate: a hook is skipped until its hash is trusted.
const CODEX_HOOK_SNIPPET: &str = r#"{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": ".*",
        "hooks": [
          { "type": "command", "command": "galdr hook", "timeout": 10 }
        ]
      }
    ]
  }
}"#;

// Cursor's native hooks (Cursor 1.7+) use a versioned container and camelCase events.
// The `postToolUse` event fires after every tool call. Cursor's payload is Claude-Code-
// styled but renames two fields (`tool_output` for `tool_response`, `conversation_id`
// for `session_id`); galdr's sensor maps them, so the same `galdr hook` command works.
const CURSOR_HOOK_SNIPPET: &str = r#"{
  "version": 1,
  "hooks": {
    "postToolUse": [
      { "type": "command", "command": "galdr hook" }
    ]
  }
}"#;

/// The step every Codex hook needs and `setup` used to omit: Codex skips an untrusted
/// hook entirely, so merging the snippet is not enough.
fn print_codex_trust_note() {
    println!();
    println!("Then TRUST the hook in Codex — it is SKIPPED until trusted:");
    println!("  • run `/hooks` in a Codex session and approve galdr's PostToolUse hook,");
    println!("  • or start Codex once with `--dangerously-bypass-hook-trust`.");
    println!("Codex stores a trusted hash per hook; an untrusted hook never fires.");
}

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
        println!("Codex PostToolUse hook is present: {}", path.display());
        println!(
            "Reminder: Codex skips an untrusted hook — confirm it is trusted (`/hooks`) if galdr records nothing."
        );
    } else {
        println!("Codex PostToolUse hook is missing: {}", path.display());
        println!("Run `galdr setup codex --print` and merge the snippet into hooks.json.");
    }
    Ok(())
}

pub fn codex_print() {
    // Merge into ~/.codex/hooks.json (hook arrays are concatenated, so galdr coexists
    // with any existing Codex hooks), then trust it — Codex ignores an untrusted hook.
    println!("{CODEX_HOOK_SNIPPET}");
    print_codex_trust_note();
}

pub fn codex_hook_configured() -> Option<bool> {
    let path = paths::codex_hooks().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&contents)
        .map(|value| has_galdr_hook(&value))
        .ok()
}

pub fn cursor_check() -> Result<()> {
    let path = paths::cursor_hooks()?;
    let Ok(contents) = std::fs::read_to_string(&path) else {
        println!("Cursor hooks file not found: {}", path.display());
        println!("Run `galdr setup cursor --print` to see the hook snippet.");
        return Ok(());
    };
    let configured = serde_json::from_str::<serde_json::Value>(&contents)
        .map(|value| has_galdr_hook(&value))
        .unwrap_or_else(|_| contents.contains("postToolUse") && contents.contains("galdr hook"));
    if configured {
        println!("Cursor postToolUse hook is present: {}", path.display());
    } else {
        println!("Cursor postToolUse hook is missing: {}", path.display());
        println!("Run `galdr setup cursor --print` and merge the snippet into hooks.json.");
    }
    Ok(())
}

pub fn cursor_print() {
    // Cursor's native hooks (since 1.7) live in `~/.cursor/hooks.json` with a `version`
    // and camelCase events. galdr's sensor reads Cursor's `postToolUse` payload via a
    // small field map (tool_output → tool_response, conversation_id → session_id), so
    // the same `galdr hook` command works — just merge this and you are done.
    println!("{CURSOR_HOOK_SNIPPET}");
}

pub fn cursor_hook_configured() -> Option<bool> {
    let path = paths::cursor_hooks().ok()?;
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
    // Split on `;`, strip a leading `then `/`do ` from each shell segment, drop
    // quotes, then match the `galdr hook` invocation. This recognizes the wrapped
    // form without being fooled by `echo galdr hook` or a neighbouring `galdr hooks`.
    command.split(';').any(|segment| {
        let mut segment = segment.trim();
        for prefix in ["then ", "do "] {
            if let Some(stripped) = segment.strip_prefix(prefix) {
                segment = stripped.trim();
            }
        }

        let normalized = segment.replace(['"', '\''], "");
        normalized == "galdr hook"
            || normalized.starts_with("galdr hook ")
            || normalized.ends_with("/galdr hook")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_path_qualified_galdr_hook_commands() {
        assert!(is_galdr_hook_command("galdr hook"));
        assert!(is_galdr_hook_command("/home/user/.cargo/bin/galdr hook"));
        assert!(is_galdr_hook_command(
            "if command -v galdr >/dev/null 2>&1; then galdr hook; elif [ -x \"$HOME/.cargo/bin/galdr\" ]; then \"$HOME/.cargo/bin/galdr\" hook; fi"
        ));
        assert!(!is_galdr_hook_command("galdr outcome list"));
        assert!(!is_galdr_hook_command("echo galdr hook"));
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
