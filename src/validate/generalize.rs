// ----------------------------------------------------------------------------
// Shared helpers (used by the distiller's render so it cannot diverge from the gate)
// ----------------------------------------------------------------------------

/// Generalizes recording-specific session data in a free-text value so it never
/// reaches an installed, shareable skill: a personal path collapses to `~/…`, a temp
/// path to `<temp path>`, an email to `<email>`. The detector above flags exactly the
/// shapes this rewrites, so a value the distiller cleans is one the gate accepts.
pub(crate) fn generalize_session_text(text: &str, home: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(home) = home.filter(|h| !h.is_empty() && h.starts_with('/')) {
        out = replace_path_prefix(&out, home, "~");
    }
    out = replace_path_prefix(&out, "/Users/", "~/USER/");
    out = replace_path_prefix(&out, "/home/", "~/USER/");
    out = collapse_user_segment(&out);
    out = replace_temp_paths(&out);
    out = replace_emails(&out);
    out
}

/// Replaces a path prefix wherever it begins a path token, keeping the remainder.
fn replace_path_prefix(text: &str, prefix: &str, with: &str) -> String {
    if !text.contains(prefix) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        out.push_str(&rest[..at]);
        out.push_str(with);
        rest = &rest[at + prefix.len()..];
    }
    out.push_str(rest);
    out
}

/// Collapses the `~/USER/<name>` placeholder left by [`replace_path_prefix`] for the
/// `/Users//home` roots down to `~`, dropping the username segment entirely.
fn collapse_user_segment(text: &str) -> String {
    const MARK: &str = "~/USER/";
    if !text.contains(MARK) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(MARK) {
        out.push_str(&rest[..at]);
        out.push('~');
        let after = &rest[at + MARK.len()..];
        // Drop the username segment up to the next path separator/terminator.
        let seg_end = after
            .find(|c: char| c == '/' || is_token_delimiter(c))
            .unwrap_or(after.len());
        let tail = &after[seg_end..];
        // Keep the separating `/` so `~/Projects/x` stays a path.
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn replace_temp_paths(text: &str) -> String {
    let mut out = text.to_string();
    for root in ["/private/tmp/", "/tmp/", "/var/folders/"] {
        out = replace_temp_root(&out, root);
    }
    out
}

fn replace_temp_root(text: &str, root: &str) -> String {
    if !text.contains(root) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(root) {
        out.push_str(&rest[..at]);
        out.push_str("<temp path>");
        let after = &rest[at + root.len()..];
        let end = after.find(is_token_delimiter).unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn replace_emails(text: &str) -> String {
    if !text.contains('@') {
        return text.to_string();
    }
    text.split_inclusive(is_token_delimiter)
        .map(|chunk| {
            let (token, delim) = split_trailing_delim(chunk);
            if is_email_token(token) {
                format!("<email>{delim}")
            } else {
                chunk.to_string()
            }
        })
        .collect()
}

fn split_trailing_delim(chunk: &str) -> (&str, &str) {
    match chunk.char_indices().last() {
        Some((i, c)) if is_token_delimiter(c) => (&chunk[..i], &chunk[i..]),
        _ => (chunk, ""),
    }
}

/// The value of a frontmatter scalar (`name:` / `description:`), unquoted, or `None`
/// if there is no properly delimited frontmatter block.
fn frontmatter_value(md: &str, key: &str) -> Option<String> {
    let body = md.trim_start_matches(['\u{feff}', ' ', '\t', '\n', '\r']);
    let after_open = body.strip_prefix("---")?;
    let opener_end = after_open.find('\n').map_or(after_open.len(), |i| i + 1);
    if !after_open[..opener_end].trim().is_empty() {
        return None;
    }
    let inner = &after_open[opener_end..];
    for line in inner.lines() {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.trim_start().strip_prefix(key)
            && let Some(value) = rest.strip_prefix(':')
        {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}
