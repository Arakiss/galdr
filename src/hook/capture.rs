fn denied_by_capture_policy(event: &span::Event, capture: &config::CaptureConfig) -> bool {
    if capture
        .deny_tools
        .iter()
        .any(|tool| tool == &event.tool_name)
    {
        return true;
    }
    if let Some(cwd) = &event.cwd
        && capture
            .deny_cwd_prefixes
            .iter()
            .any(|prefix| cwd.starts_with(prefix))
    {
        return true;
    }
    false
}

/// Recursively replaces base64 image data with a small marker, in place. It strips
/// **only with image context** — never generic base64 — so it can't silently corrupt
/// an arbitrary tool's payload (spans are append-only; a wrong strip is irreversible).
/// Image context means: an `image` content block (`type: image`, or a `media_type` /
/// `mimeType` of `image/*`), an image-ish key (`image`, `image_url`, `screenshot`),
/// or a `data:image/…;base64,…` data URI. The standard Computer Use screenshot —
/// `{"type":"image","source":{"type":"base64","media_type":"image/png","data":"…"}}`
/// — is caught; the action fields (`action`, `coordinate`, `text`) are untouched.
fn strip_screenshots(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let image_ctx = is_image_context(map);
            for (key, v) in map.iter_mut() {
                let strip = match v.as_str() {
                    Some(s) if is_data_uri_image(s) => true,
                    Some(s) if (image_ctx || is_image_key(key)) && is_base64ish(s) => true,
                    _ => false,
                };
                if strip {
                    let bytes = v.as_str().map(str::len).unwrap_or(0);
                    *v = serde_json::json!(format!("[galdr stripped screenshot: {bytes} bytes]"));
                } else {
                    strip_screenshots(v);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                strip_screenshots(item);
            }
        }
        _ => {}
    }
}

/// Whether this object is (or directly describes) an image content block.
fn is_image_context(map: &serde_json::Map<String, serde_json::Value>) -> bool {
    let s = |k: &str| map.get(k).and_then(|v| v.as_str());
    s("type") == Some("image")
        || ["media_type", "mimeType", "mime_type"]
            .iter()
            .any(|k| s(k).is_some_and(|m| m.starts_with("image/")))
}

fn is_image_key(key: &str) -> bool {
    matches!(
        key,
        "image" | "image_url" | "imageUrl" | "screenshot" | "img"
    )
}

fn is_data_uri_image(s: &str) -> bool {
    s.starts_with("data:image/")
}

/// A cheap base64 check (base64 and base64url), long enough to be a real image, not a
/// short token. Only consulted once image context is already established.
fn is_base64ish(s: &str) -> bool {
    if s.len() < 64 {
        return false;
    }
    let ok = s
        .bytes()
        .filter(|b| {
            b.is_ascii_alphanumeric()
                || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_' | b'\n' | b'\r')
        })
        .count();
    ok as f64 / s.len() as f64 > 0.95
}

/// Saves any image blobs in this event as ephemeral PNG frames under
/// `~/.galdr/frames/<rec_id>/`. Best-effort and silent: a failed frame never affects
/// the recording (the span is the truth; frames are disposable authoring scaffolding).
fn save_frames(rec_id: &str, seq: u64, input: &serde_json::Value, response: &serde_json::Value) {
    let Ok(dir) = paths::frames_dir(rec_id) else {
        return;
    };
    write_image_blobs(&dir, seq, input, response);
}

/// Collects image blobs from `input`/`response`, decodes them, and writes one PNG per
/// blob to `dir` (`<seq>.png`, `<seq>-1.png`, …). Returns how many were written. Split
/// from [`save_frames`] so the path resolution is injectable in tests.
fn write_image_blobs(
    dir: &std::path::Path,
    seq: u64,
    input: &serde_json::Value,
    response: &serde_json::Value,
) -> usize {
    let mut blobs = Vec::new();
    collect_image_blobs(input, &mut blobs);
    collect_image_blobs(response, &mut blobs);
    if blobs.is_empty() {
        return 0;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return 0;
    }
    let mut written = 0;
    for (i, b64) in blobs.iter().enumerate() {
        let Some(bytes) = decode_base64(b64) else {
            continue;
        };
        let name = if i == 0 {
            format!("{seq:04}.png")
        } else {
            format!("{seq:04}-{i}.png")
        };
        if std::fs::write(dir.join(name), bytes).is_ok() {
            written += 1;
        }
    }
    written
}

/// Read-only twin of [`strip_screenshots`]: walks the value with the same image-context
/// rules and collects the base64 payloads (data-URI prefix removed) instead of replacing
/// them. Keeping the detection identical means a frame is saved for exactly what is
/// stripped — never an arbitrary string.
fn collect_image_blobs(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            let image_ctx = is_image_context(map);
            for (key, v) in map {
                match v.as_str() {
                    Some(s) if is_data_uri_image(s) => {
                        if let Some((_, b64)) = s.split_once(',') {
                            out.push(b64.to_string());
                        }
                    }
                    Some(s) if (image_ctx || is_image_key(key)) && is_base64ish(s) => {
                        out.push(s.to_string());
                    }
                    _ => collect_image_blobs(v, out),
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_image_blobs(item, out);
            }
        }
        _ => {}
    }
}

/// Decodes standard or URL-safe base64 (no external crate — fewer deps, no supply-chain
/// surface for a defensive local tool). Skips padding and whitespace; returns `None` on
/// any out-of-alphabet byte so a malformed blob writes no frame rather than garbage.
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        buf = (buf << 6) | sextet(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Normalizes the tool result across harnesses. Claude Code and Codex send
/// `tool_response` (a JSON object); Cursor sends `tool_output` (the same payload, but
/// JSON-stringified). Prefer a present `tool_response`; otherwise adopt `tool_output`,
/// parsed back into JSON when it is a valid JSON string (else kept verbatim).
fn cursor_response(
    tool_response: serde_json::Value,
    tool_output: Option<serde_json::Value>,
) -> serde_json::Value {
    if !tool_response.is_null() {
        return tool_response;
    }
    match tool_output {
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str(&s).unwrap_or(serde_json::Value::String(s))
        }
        Some(other) => other,
        None => serde_json::Value::Null,
    }
}

fn apply_response_cap(event: &mut span::Event, max_chars: Option<usize>) {
    let Some(max_chars) = max_chars else {
        return;
    };
    let raw = event.tool_response.to_string();
    if raw.chars().count() <= max_chars {
        return;
    }
    let preview: String = raw.chars().take(max_chars).collect();
    event.tool_response = serde_json::json!({
        "galdr_truncated": true,
        "original_chars": raw.chars().count(),
        "preview": preview,
    });
}
