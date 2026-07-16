fn browser_content_script(port: u16) -> String {
    format!(
        r##"(function () {{
  if (window.__galdrObserveInstalled) return;
  window.__galdrObserveInstalled = true;

  const endpoint = "http://127.0.0.1:{port}/event";
  const sentNavigate = new Set();

  function source() {{
    return {{
      kind: "browser",
      url: window.location.href,
      title: document.title || null
    }};
  }}

  function textOf(el) {{
    if (!el) return null;
    const tag = el.tagName ? el.tagName.toLowerCase() : "";
    if (tag === "input" || tag === "textarea" || tag === "select") return null;
    const value = (el.innerText || el.textContent || "").trim();
    return value ? value.slice(0, 120) : null;
  }}

  function cssFor(el) {{
    if (!el || !el.tagName) return null;
    if (el.id) return "#" + CSS.escape(el.id);
    const testId = el.getAttribute("data-testid") || el.getAttribute("data-test");
    if (testId) return "[data-testid=\"" + CSS.escape(testId) + "\"]";
    const tag = el.tagName.toLowerCase();
    const name = el.getAttribute("name");
    if (name) return tag + "[name=\"" + CSS.escape(name) + "\"]";
    return tag;
  }}

  function targetFor(node) {{
    const el = node && node.closest ? node.closest("button,a,input,textarea,select,label,[role],[data-testid],[data-test]") : node;
    if (!el || !el.getAttribute) return null;
    const role = el.getAttribute("role") || (el.tagName ? el.tagName.toLowerCase() : null);
    const aria = el.getAttribute("aria-label");
    const label = labelsFor(el);
    const placeholder = el.getAttribute("placeholder");
    const text = textOf(el);
    const css = cssFor(el);
    const primary = label
      ? {{ kind: "label", value: label }}
      : placeholder
        ? {{ kind: "placeholder", value: placeholder }}
        : aria
          ? {{ kind: "role", role: role || "element", name: aria }}
          : css
            ? {{ kind: "css", value: css }}
            : {{ kind: "css", value: "element" }};
    return {{
      primary,
      role,
      name: aria || label || text || placeholder || null,
      text,
      label,
      placeholder,
      element_summary: el.tagName ? el.tagName.toLowerCase() : null
    }};
  }}

  function labelsFor(el) {{
    if (!el) return null;
    if (el.labels && el.labels.length) {{
      const label = Array.from(el.labels).map(l => (l.innerText || l.textContent || "").trim()).find(Boolean);
      if (label) return label.slice(0, 120);
    }}
    const id = el.getAttribute && el.getAttribute("id");
    if (id) {{
      const label = document.querySelector("label[for='" + CSS.escape(id) + "']");
      if (label) {{
        const text = (label.innerText || label.textContent || "").trim();
        if (text) return text.slice(0, 120);
      }}
    }}
    const closest = el.closest && el.closest("label");
    if (closest) {{
      const text = (closest.innerText || closest.textContent || "").trim();
      if (text) return text.slice(0, 120);
    }}
    return null;
  }}

  function emit(action, target, value, verificationHint) {{
    const event = {{
      ts: new Date().toISOString(),
      action,
      source: source(),
      target: target || null,
      value: value || null,
      verification_hint: verificationHint || null
    }};
    const body = JSON.stringify(event);
    fetch(endpoint, {{
      method: "POST",
      body,
      keepalive: true
    }}).catch(() => {{
      if (navigator.sendBeacon) {{
        try {{ navigator.sendBeacon(endpoint, body); }} catch (_) {{}}
      }}
    }});
  }}

  function emitNavigate() {{
    const key = window.location.href + "::" + document.title;
    if (sentNavigate.has(key)) return;
    sentNavigate.add(key);
    emit("human.browser.navigate", null, null, null);
  }}

  function inputValueFor(el) {{
    const type = (el.getAttribute("type") || "").toLowerCase();
    const name = [el.getAttribute("name"), el.getAttribute("id"), labelsFor(el), el.getAttribute("placeholder")]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    if (type === "password") {{
      return {{ policy: "omitted", reason: "password-field" }};
    }}
    if (/password|passwd|token|secret|api[_-]?key|authorization|credential|private[_-]?key/.test(name)) {{
      return {{ policy: "redacted", kind: "sensitive-field", chars: (el.value || "").length }};
    }}
    return {{ policy: "redacted", kind: "text", chars: (el.value || "").length }};
  }}

  document.addEventListener("DOMContentLoaded", emitNavigate, true);
  window.addEventListener("load", emitNavigate, true);
  setTimeout(emitNavigate, 0);
  setTimeout(emitNavigate, 500);
  document.addEventListener("click", event => {{
    emit("human.browser.click", targetFor(event.target), null, null);
  }}, true);
  document.addEventListener("input", event => {{
    const el = event.target;
    if (!el || !("value" in el)) return;
    emit("human.browser.input", targetFor(el), inputValueFor(el), null);
  }}, true);
  document.addEventListener("change", event => {{
    const el = event.target;
    if (!el || !el.tagName) return;
    const tag = el.tagName.toLowerCase();
    if (tag === "select") {{
      const selected = el.options && el.selectedIndex >= 0 ? el.options[el.selectedIndex].text : el.value;
      emit("human.browser.select", targetFor(el), {{ policy: "literal", value: selected || el.value || "" }}, null);
    }} else if (el.type === "checkbox" || el.type === "radio") {{
      emit("human.browser.check", targetFor(el), {{ policy: "literal", value: el.checked ? "true" : "false" }}, null);
    }}
  }}, true);
  document.addEventListener("keydown", event => {{
    const printable = event.key && event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey;
    if (printable) return;
    const combo = [
      event.metaKey ? "Meta" : null,
      event.ctrlKey ? "Ctrl" : null,
      event.altKey ? "Alt" : null,
      event.shiftKey ? "Shift" : null,
      event.key
    ].filter(Boolean).join("+");
    emit("human.browser.key", targetFor(event.target), {{ policy: "literal", value: combo }}, null);
  }}, true);
}})();
"##
    )
}
