//! Tiny TTY-aware styling: color and symbols for a human at a terminal, plain text for
//! a pipe or an agent. The CLI is dual-audience — a person reading a terminal and an
//! agent reading a pipe — so styling is applied only when stdout is an interactive
//! terminal and `NO_COLOR` is unset. No dependencies; just ANSI when it is welcome.

use std::io::IsTerminal;

/// Whether to emit ANSI styling: stdout is a terminal and `NO_COLOR` is not set. An
/// agent, a pipe, or a redirect gets clean, unstyled text it can parse.
pub fn enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

fn wrap(code: &str, s: &str) -> String {
    if enabled() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Bold.
pub fn bold(s: &str) -> String {
    wrap("1", s)
}

/// Dim / secondary text.
pub fn dim(s: &str) -> String {
    wrap("2", s)
}

/// The galdr accent (a calm teal).
pub fn accent(s: &str) -> String {
    wrap("38;5;43", s)
}

/// Success green.
pub fn green(s: &str) -> String {
    wrap("32", s)
}

/// Attention red (e.g. the live-recording dot).
pub fn red(s: &str) -> String {
    wrap("31", s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstyled_when_no_color_is_set() {
        // SAFETY: single-threaded test; we set then restore the env var.
        unsafe { std::env::set_var("NO_COLOR", "1") };
        assert!(!enabled());
        assert_eq!(bold("x"), "x", "no ANSI codes when NO_COLOR is set");
        assert_eq!(accent("hi"), "hi");
        unsafe { std::env::remove_var("NO_COLOR") };
    }
}
