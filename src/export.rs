//! Recording export helpers.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::{catalog, paths, record, span};

pub fn export_recording(id: &str, out: &Path, include_raw: bool, redact: bool) -> Result<()> {
    std::fs::create_dir_all(out)
        .with_context(|| format!("could not create export directory {}", out.display()))?;

    let rec_path = paths::recording_file(id)?;
    let recording_json = std::fs::read_to_string(&rec_path)
        .with_context(|| format!("recording {id} not found. Did you run `galdr rec stop`?"))?;
    let recording: record::Recording = serde_json::from_str(&recording_json)?;
    // With --redact, every exported file is scrubbed, not just the raw span: a step
    // summary (a Bash command) or a usage note can carry a secret just as a raw
    // payload can. An export marketed as redacted must actually be redacted.
    write_json(&out.join("recording.json"), &recording, redact)?;

    let conn = catalog::open_in_memory_indexed()?;
    if let Some(detail) = catalog::show_recording(&conn, id)? {
        let mut steps = String::new();
        steps.push_str("# galdr export\n\n");
        steps.push_str(&format!(
            "- rec_id: `{}`\n- name: `{}`\n- steps: {}\n\n",
            recording.rec_id,
            maybe_redact_text(&recording.name, redact),
            detail.steps.len()
        ));
        steps.push_str("## Steps\n\n");
        for step in detail.steps {
            steps.push_str(&format!(
                "{}. **{}** — {}\n",
                step.seq + 1,
                step.tool_name,
                maybe_redact_text(&step.summary, redact)
            ));
        }
        std::fs::write(out.join("steps.md"), steps)?;
    }

    let skills: Vec<_> = catalog::list_skills(&conn)?
        .into_iter()
        .filter(|skill| skill.rec_id.as_deref() == Some(id))
        .collect();
    write_json(&out.join("skills.json"), &skills, redact)?;
    let usages: Vec<_> = catalog::list_skill_usage(&conn, None)?
        .into_iter()
        .filter(|usage| usage.rec_id == id)
        .collect();
    write_json(&out.join("usage.json"), &usages, redact)?;

    let outcomes: Vec<_> = catalog::list_skill_outcomes(&conn, None)?
        .into_iter()
        .filter(|outcome| outcome.rec_id.as_deref() == Some(id))
        .collect();
    write_json(&out.join("outcomes.json"), &outcomes, redact)?;

    if include_raw || redact {
        if include_raw && !redact {
            eprintln!("warning: exporting raw tool payloads; treat the output as sensitive");
        }
        let span_path = paths::span_file(id)?;
        let events = span::read_span(&span_path)?;
        let file_name = if redact {
            "raw.redacted.jsonl"
        } else {
            "raw.jsonl"
        };
        let mut file = std::fs::File::create(out.join(file_name))?;
        for mut event in events {
            if redact {
                redact_value(&mut event.tool_input);
                redact_value(&mut event.tool_response);
            }
            writeln!(file, "{}", serde_json::to_string(&event)?)?;
        }
    }

    println!("export written to {}", out.display());
    Ok(())
}

/// Writes a value as pretty JSON, redacting it first when `redact` is set.
fn write_json<T: serde::Serialize>(path: &Path, value: &T, redact: bool) -> Result<()> {
    let mut json = serde_json::to_value(value)?;
    if redact {
        redact_value(&mut json);
    }
    std::fs::write(path, serde_json::to_string_pretty(&json)?)?;
    Ok(())
}

/// Scrubs secret-shaped tokens from a free-text string when `redact` is set.
fn maybe_redact_text(text: &str, redact: bool) -> String {
    if redact {
        redact_secrets_in_text(text).unwrap_or_else(|| text.to_string())
    } else {
        text.to_string()
    }
}

fn redact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_sensitive_key(key) {
                    *value = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_value(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_value(value);
            }
        }
        // A secret is just as likely to ride inside a string value — a token pasted
        // into a Bash command, a key in a URL — where no sensitive *key* guards it.
        // Catch the unmistakable shapes so key-based redaction is not the only net.
        serde_json::Value::String(text) => {
            if let Some(clean) = redact_secrets_in_text(text) {
                *text = clean;
            }
        }
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "token",
        "secret",
        "api_key",
        "apikey",
        "authorization",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

/// Well-known secret token shapes. Conservative on purpose: every prefix here is a
/// credential format, so false positives are near zero while the worst leaks (a
/// provider key pasted into a command) are caught.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",         // OpenAI / Anthropic-style API keys
    "sk_live_",    // Stripe live
    "sk_test_",    // Stripe test
    "ghp_",        // GitHub personal access token
    "gho_",        // GitHub OAuth
    "ghu_",        // GitHub user-to-server
    "ghs_",        // GitHub server-to-server
    "github_pat_", // GitHub fine-grained PAT
    "xoxb-",       // Slack bot token
    "xoxp-",       // Slack user token
    "xoxa-",       // Slack app token
    "xoxr-",       // Slack refresh token
    "AKIA",        // AWS access key id
    "ASIA",        // AWS temporary access key id
    "AIza",        // Google API key
    "ya29.",       // Google OAuth access token
    "glpat-",      // GitLab personal access token
    "npm_",        // npm token
    "shpat_",      // Shopify access token
    "shpss_",      // Shopify shared secret
    "eyJ",         // JWT (base64 of {"alg...) — header of almost every JWT
    "-----BEGIN",  // PEM key block opener (the whole block is also redacted below)
];

/// Scrubs secret-shaped tokens from free text, always returning a string (the input
/// unchanged if nothing matched). Shared with the distiller so a secret typed into a
/// GUI or pasted into a command never lands in an installed, shareable `SKILL.md`.
pub(crate) fn redact_text(text: &str) -> String {
    redact_secrets_in_text(text).unwrap_or_else(|| text.to_string())
}

/// Replaces any whitespace-delimited token that matches a known secret shape with
/// `[REDACTED]`, leaving the rest of the string intact so the step stays readable.
/// Returns `None` when nothing matched (so callers can skip the allocation).
pub(crate) fn redact_secrets_in_text(text: &str) -> Option<String> {
    // A PEM block spans multiple lines; redact the whole body, not just the marker.
    if let Some(pem_free) = redact_pem_blocks(text) {
        // Recurse to also catch token-shaped secrets in the remaining text.
        return Some(redact_secrets_in_text(&pem_free).unwrap_or(pem_free));
    }
    if !text
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ':' | ',' | '(' | ')'))
        .any(looks_like_secret)
    {
        return None;
    }
    let redacted: String = text
        .split_inclusive(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '\'' | '=' | ':' | ',' | '(' | ')')
        })
        .map(|chunk| {
            // Split the trailing delimiter (if any) off the token before matching.
            let (token, delim) = match chunk.char_indices().last() {
                Some((i, c))
                    if c.is_whitespace()
                        || matches!(c, '"' | '\'' | '=' | ':' | ',' | '(' | ')') =>
                {
                    (&chunk[..i], &chunk[i..])
                }
                _ => (chunk, ""),
            };
            if looks_like_secret(token) {
                format!("[REDACTED]{delim}")
            } else {
                chunk.to_string()
            }
        })
        .collect();
    Some(redacted)
}

fn looks_like_secret(token: &str) -> bool {
    SECRET_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

/// Reports whether `text` carries a secret-shaped token or PEM block, without
/// altering it. Shared with the validation gate, which must *flag* a leaked secret
/// (and refuse to install) rather than silently redact it the way an export does.
pub(crate) fn contains_secret(text: &str) -> bool {
    redact_secrets_in_text(text).is_some()
}

/// Replaces the entire body of any PEM block (`-----BEGIN … -----END …-----`) with
/// a single `[REDACTED]`, so a private key is never disclosed past its first line.
/// Returns `None` if no block is present.
fn redact_pem_blocks(text: &str) -> Option<String> {
    const BEGIN: &str = "-----BEGIN";
    if !text.contains(BEGIN) {
        return None;
    }
    let mut out = String::new();
    let mut rest = text;
    let mut redacted_any = false;
    while let Some(start) = rest.find(BEGIN) {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        // A PEM block ends at the line after its `-----END …-----`.
        if let Some(end_marker) = after.find("-----END")
            && let Some(end_dashes) = after[end_marker..].find("-----\n")
        {
            let block_end = end_marker + end_dashes + "-----".len();
            out.push_str("[REDACTED PEM BLOCK]");
            rest = &after[block_end..];
            redacted_any = true;
        } else if let Some(end_marker) = after.find("-----END")
            && let Some(end_dashes) = after[end_marker..].rfind("-----")
        {
            // Final block with no trailing newline.
            let block_end = end_marker + end_dashes + "-----".len();
            out.push_str("[REDACTED PEM BLOCK]");
            rest = &after[block_end..];
            redacted_any = true;
        } else {
            out.push_str(after);
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    redacted_any.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_a_secret_embedded_in_a_command_string() {
        let mut v = serde_json::json!({ "command": "curl -H 'Authorization: Bearer sk-abc123XYZ' https://api" });
        redact_value(&mut v);
        let s = v["command"].as_str().unwrap();
        assert!(s.contains("[REDACTED]"), "got: {s}");
        assert!(!s.contains("sk-abc123XYZ"));
        // Non-secret tokens around it survive.
        assert!(s.contains("curl"));
    }

    #[test]
    fn leaves_ordinary_strings_untouched() {
        let mut v = serde_json::json!({ "command": "git status --short && cargo build" });
        let before = v.clone();
        redact_value(&mut v);
        assert_eq!(v, before, "no secret shape, nothing to redact");
    }

    #[test]
    fn still_redacts_by_sensitive_key() {
        let mut v = serde_json::json!({ "api_key": "whatever-it-is", "ok": true });
        redact_value(&mut v);
        assert_eq!(v["api_key"], serde_json::json!("[REDACTED]"));
        assert_eq!(v["ok"], serde_json::json!(true));
    }

    #[test]
    fn recognizes_common_secret_prefixes() {
        for token in [
            "ghp_0123456789",
            "AKIAIOSFODNN7EXAMPLE",
            "xoxb-12345",
            "glpat-abcdefghij",
            "ya29.aBcDeF",
            "npm_0123456789",
            "eyJhbGciOiJIUzI1NiJ9",
        ] {
            assert!(
                looks_like_secret(token),
                "{token} should look like a secret"
            );
        }
        assert!(!looks_like_secret("git"));
        assert!(!looks_like_secret("status"));
    }

    #[test]
    fn redacts_a_whole_pem_private_key_block() {
        let text = "key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEoQIDsecretAQAB\n-----END RSA PRIVATE KEY-----\ndone";
        let out = redact_secrets_in_text(text).expect("PEM should trigger redaction");
        assert!(out.contains("[REDACTED PEM BLOCK]"));
        assert!(!out.contains("MIIEoQID"));
        assert!(out.contains("done"));
    }

    #[test]
    fn redact_text_helper_only_acts_when_asked() {
        let secret = "run with token ghp_SECRETabc123";
        assert!(maybe_redact_text(secret, false).contains("ghp_SECRETabc123"));
        let redacted = maybe_redact_text(secret, true);
        assert!(!redacted.contains("ghp_SECRETabc123"));
        assert!(redacted.contains("[REDACTED]"));
    }
}
