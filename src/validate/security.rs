/// Runs the gate over a `SKILL.md`. Pure: decides severity per axis but never blocks
/// — the caller does, via [`ValidationReport::has_blocking`].
pub fn validate_skill(md: &str, ctx: &ValidationCtx) -> ValidationReport {
    let mut report = ValidationReport::default();
    security_findings(md, ctx.home.as_deref(), &mut report);
    if !ctx.draft {
        practicality_findings(md, &mut report);
    }
    optimization_findings(md, &mut report);
    report
}

// ----------------------------------------------------------------------------
// Security
// ----------------------------------------------------------------------------

fn security_findings(md: &str, home: Option<&str>, report: &mut ValidationReport) {
    let mut in_fence = false;
    for (idx, line) in md.lines().enumerate() {
        let lineno = idx + 1;
        if is_fence_delimiter(line) {
            in_fence = !in_fence;
            continue;
        }

        // Secrets and PEM blocks: blocking, no bypass, fence or not — a leaked key is
        // a leaked key wherever it sits.
        if crate::export::contains_secret(line) {
            report.push(
                Severity::Error,
                Category::Security,
                "SEC_SECRET",
                "a secret-shaped token or key block is present; remove it before installing",
                Some(lineno),
            );
        }

        // Personal / PII paths and emails: blocking, no bypass.
        for hit in pii_hits(line, home) {
            report.push(
                Severity::Error,
                Category::Security,
                hit.code,
                hit.message,
                Some(lineno),
            );
        }

        // Prompt-injection shapes: surfaced, never blocking on their own.
        if let Some(message) = injection_hit(line) {
            report.push(
                Severity::Warn,
                Category::Security,
                "SEC_INJECTION",
                message,
                Some(lineno),
            );
        }

        // Dangerous commands, aware of code context.
        let inline = inline_code_mask(line);
        for hit in dangerous_hits(line) {
            let in_inline = inline.get(hit.offset).copied().unwrap_or(false);
            let documented = in_fence || in_inline;
            match (hit.fatal, documented) {
                // Catastrophic, outside a fenced block: a literal foot-gun the agent
                // might run. Blocks, no bypass.
                (true, false) => report.push(
                    Severity::Error,
                    Category::Security,
                    "SEC_DANGER_CMD_FATAL",
                    format!("catastrophic command in prose: {}", hit.message),
                    Some(lineno),
                ),
                // Catastrophic but inside a fence/inline code: documented, not asserted.
                (true, true) => report.push(
                    Severity::Warn,
                    Category::Security,
                    "SEC_DANGER_CMD",
                    format!("catastrophic command (documented): {}", hit.message),
                    Some(lineno),
                ),
                // Legitimately dangerous in prose: warn (blocks only under --strict).
                (false, false) => report.push(
                    Severity::Warn,
                    Category::Security,
                    "SEC_DANGER_CMD",
                    format!("dangerous command in prose: {}", hit.message),
                    Some(lineno),
                ),
                // Legitimately dangerous but inside code: documented, suppressed.
                (false, true) => {}
            }
        }
    }
}

/// A line that opens or closes a fenced code block (``` or ~~~, any indent).
fn is_fence_delimiter(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// A per-byte mask: `true` where the byte sits inside an inline-code span (between a
/// pair of backticks on the line). Used to suppress documented dangerous commands.
fn inline_code_mask(line: &str) -> Vec<bool> {
    let mut mask = vec![false; line.len()];
    let mut inside = false;
    for (i, c) in line.char_indices() {
        if c == '`' {
            inside = !inside;
            continue;
        }
        for b in i..i + c.len_utf8() {
            if b < mask.len() {
                mask[b] = inside;
            }
        }
    }
    mask
}

struct PiiHit {
    code: &'static str,
    message: String,
}

/// Personal-path and email hits on one line. A `~/…` path is already relative and
/// clean (so the self-skill's `~/.galdr` does not false-positive); only absolute
/// personal/session roots and emails are flagged.
fn pii_hits(line: &str, home: Option<&str>) -> Vec<PiiHit> {
    let mut hits = Vec::new();
    let mut flagged_path = false;
    const ROOTS: &[&str] = &[
        "/Users/",
        "/home/",
        "/private/tmp/",
        "/tmp/",
        "/var/folders/",
    ];
    for root in ROOTS {
        if let Some(pos) = line.find(root) {
            // For /Users and /home, require an actual user segment after the root so
            // the bare root alone is not what trips it.
            let after = &line[pos + root.len()..];
            let has_segment = after
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace() && c != '/');
            if (*root == "/Users/" || *root == "/home/") && !has_segment {
                continue;
            }
            flagged_path = true;
            hits.push(PiiHit {
                code: "SEC_PII_PATH",
                message: format!(
                    "personal or session path under `{root}`; generalize it to `~/…` or drop it"
                ),
            });
        }
    }
    // The user's actual home directory, in case it is not under a standard root
    // (a relocated `$HOME`, a custom layout). Skipped if a standard root already
    // matched, so the same path is not flagged twice.
    if !flagged_path
        && let Some(home) = home.filter(|h| h.starts_with('/') && h.len() > 1)
        && line.contains(home)
    {
        hits.push(PiiHit {
            code: "SEC_PII_PATH",
            message: "personal path under your home directory; generalize it to `~/…`".to_string(),
        });
    }
    if line.split(is_token_delimiter).any(is_email_token) {
        hits.push(PiiHit {
            code: "SEC_PII_EMAIL",
            message: "an email address is present; remove personal contact data".to_string(),
        });
    }
    hits
}

fn is_token_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '`' | '(' | ')' | ',' | ';' | '<' | '>' | '='
        )
}

/// A conservative email check: `local@domain.tld`, with a real dotted domain and a
/// 2+ letter TLD, so `@scope/pkg` and a bare `@handle` do not false-positive.
fn is_email_token(token: &str) -> bool {
    let token = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let Some(at) = token.find('@') else {
        return false;
    };
    let (local, domain) = (&token[..at], &token[at + 1..]);
    if local.is_empty()
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-'))
    {
        return false;
    }
    let Some(dot) = domain.rfind('.') else {
        return false;
    };
    let (host, tld) = (&domain[..dot], &domain[dot + 1..]);
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
}

struct DangerHit {
    offset: usize,
    fatal: bool,
    message: String,
}

/// Dangerous-command hits on one line, each tagged catastrophic (`fatal`) or merely
/// dangerous. Context (fence/inline) and final severity are decided by the caller.
fn dangerous_hits(line: &str) -> Vec<DangerHit> {
    let mut hits = Vec::new();

    // `rm` with recursive+force: catastrophic when aimed at a filesystem/home root,
    // otherwise a targeted (but still dangerous) delete.
    for flag in ["rm -rf", "rm -fr", "rm -r -f", "rm -f -r"] {
        let mut from = 0;
        while let Some(rel) = line[from..].find(flag) {
            let at = from + rel;
            let target = line[at + flag.len()..].trim_start();
            let fatal = is_catastrophic_target(target);
            hits.push(DangerHit {
                offset: at,
                fatal,
                message: if fatal {
                    "recursive force-delete of a filesystem or home root".to_string()
                } else {
                    "recursive force-delete (`rm -rf`)".to_string()
                },
            });
            from = at + flag.len();
        }
    }

    // Fork bomb.
    if let Some(at) = line.find(":(){").or_else(|| line.find(":|:&")) {
        hits.push(DangerHit {
            offset: at,
            fatal: true,
            message: "fork bomb".to_string(),
        });
    }
    // Filesystem-making and raw device writes.
    if let Some(at) = line.find("mkfs") {
        hits.push(DangerHit {
            offset: at,
            fatal: true,
            message: "filesystem creation (`mkfs`)".to_string(),
        });
    }
    if line.contains("dd ")
        && let Some(at) = ["of=/dev/sd", "of=/dev/disk", "of=/dev/nvme", "of=/dev/hd"]
            .iter()
            .find_map(|p| line.find(p))
    {
        hits.push(DangerHit {
            offset: at,
            fatal: true,
            message: "raw write to a block device (`dd of=/dev/…`)".to_string(),
        });
    }

    // Legitimately dangerous, but common in real ops work — warn only in prose.
    push_warn(
        &mut hits,
        line,
        "push --force",
        "force-push (`git push --force`)",
    );
    push_warn(&mut hits, line, "push -f", "force-push (`git push -f`)");
    push_warn(&mut hits, line, "sudo ", "elevated privileges (`sudo`)");
    push_warn(
        &mut hits,
        line,
        "chmod 777",
        "world-writable permissions (`chmod 777`)",
    );
    if (line.contains("curl") || line.contains("wget"))
        && let Some(at) = ["| sh", "|sh", "| bash", "|bash"]
            .iter()
            .find_map(|p| line.find(p))
    {
        hits.push(DangerHit {
            offset: at,
            fatal: false,
            message: "piping a download straight into a shell (`curl | sh`)".to_string(),
        });
    }

    hits
}

fn push_warn(hits: &mut Vec<DangerHit>, line: &str, needle: &str, message: &str) {
    if let Some(at) = line.find(needle) {
        hits.push(DangerHit {
            offset: at,
            fatal: false,
            message: message.to_string(),
        });
    }
}

/// Whether an `rm -rf` target is a filesystem or home root (catastrophic), versus a
/// specific subdirectory (dangerous but targeted).
fn is_catastrophic_target(target: &str) -> bool {
    let first = target.split(is_token_delimiter).next().unwrap_or("");
    // The whole filesystem: `/`, `//`, `/*`, `/ *`…
    if first.starts_with('/') && first.trim_start_matches(['/', '*']).is_empty() {
        return true;
    }
    // The whole home directory.
    matches!(first, "~" | "~/" | "$HOME" | "$HOME/")
        || (first.starts_with("~/") && first[2..].chars().all(|c| c == '*'))
        || (first.starts_with("$HOME/") && first["$HOME/".len()..].chars().all(|c| c == '*'))
}

/// A prompt-injection shape worth surfacing. Conservative: well-known override
/// phrases, role-token markers, and invisible bidi/zero-width control characters.
fn injection_hit(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    const PHRASES: &[&str] = &[
        "ignore previous instructions",
        "ignore all previous instructions",
        "disregard previous instructions",
        "you are now",
        "<|im_start|>",
        "<|im_end|>",
    ];
    for phrase in PHRASES {
        if lower.contains(phrase) {
            return Some(format!("possible prompt-injection phrase: \"{phrase}\""));
        }
    }
    if line.chars().any(is_suspicious_invisible) {
        return Some("invisible bidi/zero-width control character".to_string());
    }
    None
}

/// Zero-width and bidirectional-override characters that have no business in a skill
/// and can hide or reorder instructions. A leading BOM is tolerated elsewhere and not
/// matched here.
fn is_suspicious_invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' // zero-width
        | '\u{202A}'..='\u{202E}'                          // bidi embeddings/overrides
        | '\u{2066}'..='\u{2069}'                          // bidi isolates
    )
}
