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

fn is_galdr_hook_command(command: &str) -> bool {
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
        assert!(is_galdr_hook_command(
            "/Users/dolores/.cargo/bin/galdr hook"
        ));
        assert!(is_galdr_hook_command(
            "if command -v galdr >/dev/null 2>&1; then galdr hook; elif [ -x \"$HOME/.cargo/bin/galdr\" ]; then \"$HOME/.cargo/bin/galdr\" hook; fi"
        ));
        assert!(!is_galdr_hook_command("galdr outcome list"));
        assert!(!is_galdr_hook_command("echo galdr hook"));
    }
}
