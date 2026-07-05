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

    let judgments = catalog::list_step_judgments(&conn, Some(id))?;
    write_json(&out.join("judgments.json"), &judgments, redact)?;

    let regression_cases: Vec<_> = catalog::list_regression_base_cases(&conn, None)?
        .into_iter()
        .filter(|case| case.rec_id == id)
        .collect();
    write_json(&out.join("regression.json"), &regression_cases, redact)?;

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
                if let Some(human) = &mut event.human {
                    redact_human_event(human);
                }
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

fn redact_human_event(human: &mut span::HumanEvent) {
    redact_human_source(&mut human.source);
    if let Some(target) = &mut human.target {
        redact_human_target(target);
    }
    if let Some(value) = &mut human.value {
        redact_human_value(value);
    }
    if let Some(hint) = &mut human.verification_hint {
        redact_string(hint);
    }
    if let Some(frame_ref) = &mut human.frame_ref {
        redact_string(frame_ref);
    }
}

fn redact_human_source(source: &mut span::HumanSource) {
    match source {
        span::HumanSource::Browser { url, title, tab_id } => {
            redact_optional_string(url);
            redact_optional_string(title);
            redact_optional_string(tab_id);
        }
        span::HumanSource::MacApp { app, window_title } => {
            redact_optional_string(app);
            redact_optional_string(window_title);
        }
    }
}

fn redact_human_target(target: &mut span::HumanTarget) {
    redact_locator(&mut target.primary);
    for locator in &mut target.alternates {
        redact_locator(locator);
    }
    redact_optional_string(&mut target.role);
    redact_optional_string(&mut target.name);
    redact_optional_string(&mut target.text);
    redact_optional_string(&mut target.label);
    redact_optional_string(&mut target.placeholder);
    redact_optional_string(&mut target.element_summary);
}

fn redact_locator(locator: &mut span::TargetLocator) {
    match locator {
        span::TargetLocator::Role { role, name } => {
            redact_string(role);
            redact_optional_string(name);
        }
        span::TargetLocator::Label { value }
        | span::TargetLocator::Placeholder { value }
        | span::TargetLocator::TestId { value }
        | span::TargetLocator::Css { value }
        | span::TargetLocator::XPath { value } => redact_string(value),
    }
}

fn redact_human_value(value: &mut span::HumanValue) {
    match value {
        span::HumanValue::Literal { value: literal } => {
            let chars = literal.chars().count();
            *value = span::HumanValue::Redacted {
                kind: "literal".to_string(),
                chars: Some(chars),
            };
        }
        span::HumanValue::Omitted { reason } => redact_string(reason),
        span::HumanValue::Redacted { kind, .. } => redact_string(kind),
    }
}

fn redact_optional_string(value: &mut Option<String>) {
    if let Some(value) = value {
        redact_string(value);
    }
}

fn redact_string(value: &mut String) {
    *value = redact_text(value);
}

fn is_sensitive_key(key: &str) -> bool {
    name_is_sensitive(key)
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
    // First a `name=value` / `name: value` pass: a credential whose *name* is
    // sensitive (an AWS secret access key, a `password=…`) has no recognizable token
    // prefix, so only the key name gives it away. Then the token-prefix pass over the
    // result, so both nets apply.
    let keyed = redact_keyed_secrets(text);
    let base = keyed.as_deref().unwrap_or(text);
    match redact_prefixed_secrets(base) {
        Some(redacted) => Some(redacted),
        None => keyed,
    }
}

/// The token-prefix net: replaces any delimiter-separated token that matches a known
/// secret shape (`sk-`, `ghp_`, a JWT header, …) with `[REDACTED]`. Returns `None`
/// when nothing matched, so a caller can skip the allocation.
fn redact_prefixed_secrets(text: &str) -> Option<String> {
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

/// Substrings that mark a credential by its *name*, for the `name=value` /
/// `name: value` pass. Specific on purpose — no bare `key` (which would match
/// `keyboard`/`monkey`) — so a sensitive name is a strong, low-false-positive signal.
const SENSITIVE_KEY_NEEDLES: &[&str] = &[
    "password",
    "passwd",
    "token",
    "secret",
    "api_key",
    "apikey",
    "authorization",
    "credential",
    "access_key",
    "private_key",
    "client_secret",
];

/// Whether a credential *name* (the left side of an assignment, or a JSON key) is
/// sensitive.
fn name_is_sensitive(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SENSITIVE_KEY_NEEDLES
        .iter()
        .any(|needle| name.contains(needle))
}

/// Redacts the value of a `name=value` or `name: value` assignment when `name` is
/// sensitive, whatever the value's shape. This is the net for a credential with no
/// recognizable prefix — the canonical case is an AWS secret access key
/// (`AWS_SECRET_ACCESS_KEY=wJalr…`), which `looks_like_secret` cannot match. It fires
/// only behind a sensitive name and only for a substantial value (≥ 8 chars), so a
/// boolean or a short flag is left alone. Returns `None` when nothing matched.
fn redact_keyed_secrets(text: &str) -> Option<String> {
    const MIN_VALUE: usize = 8;
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut changed = false;
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        // `name=value` / `name:value` inside one word.
        if let Some((key, sep, value)) = split_assignment(word)
            && name_is_sensitive(key)
            && value_is_substantial(value, MIN_VALUE)
        {
            out.push(format!("{key}{sep}[REDACTED]"));
            changed = true;
            i += 1;
            continue;
        }
        // `name:` / `name=` then the value as the next word.
        let bare = word.trim_end_matches([':', '=']);
        if bare.len() < word.len()
            && name_is_sensitive(bare)
            && let Some(next) = words.get(i + 1)
            && value_is_substantial(next, MIN_VALUE)
        {
            out.push(word.to_string());
            out.push("[REDACTED]".to_string());
            changed = true;
            i += 2;
            continue;
        }
        out.push(word.to_string());
        i += 1;
    }
    changed.then(|| out.join(" "))
}

/// Splits `name=value` / `name:value` at the first `=` or `:`, returning
/// `(name, separator, value)`. `None` if there is no separator or the name is empty.
fn split_assignment(word: &str) -> Option<(&str, &str, &str)> {
    let idx = word.find(['=', ':'])?;
    if idx == 0 {
        return None;
    }
    Some((&word[..idx], &word[idx..idx + 1], &word[idx + 1..]))
}

/// Whether a value is worth redacting: substantial and not already redacted. Quotes
/// and backticks are stripped before measuring its length.
fn value_is_substantial(value: &str, min: usize) -> bool {
    let v = value.trim_matches(|c| matches!(c, '"' | '\'' | '`'));
    v.len() >= min && v != "[REDACTED]"
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
    fn redacts_a_sensitive_keyed_value_with_no_prefix() {
        // The canonical gap: an AWS secret access key has no recognizable prefix, so
        // only the sensitive key *name* gives it away. The token-prefix net misses it;
        // the name=value net must catch it.
        let leak = "export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        assert!(contains_secret(leak), "keyed secret should be detected");
        let redacted = redact_text(leak);
        assert!(redacted.contains("[REDACTED]"));
        assert!(!redacted.contains("wJalrXUtnFEMI"));
        // The key name survives so the step still reads.
        assert!(redacted.contains("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn redacts_a_sensitive_value_in_the_next_word() {
        let leak = "password: hunter2-very-long-passphrase";
        assert!(contains_secret(leak));
        assert!(!redact_text(leak).contains("hunter2-very-long-passphrase"));
    }

    #[test]
    fn leaves_short_or_non_sensitive_assignments_alone() {
        // A non-sensitive name, even with a long value, is not a secret.
        assert!(!contains_secret(
            "output_path=/Users/me/build/artifact.tar.gz"
        ));
        // A sensitive name with a trivial value (a boolean/flag) is not redacted.
        assert!(!contains_secret("secret=on"));
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
