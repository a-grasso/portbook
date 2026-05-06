//! Terminal-width helpers. Pure string transforms; no I/O beyond
//! reading the terminal size.

pub(super) fn term_width() -> usize {
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        w as usize
    } else {
        std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120)
    }
}

/// Visible width: ignores ANSI escape sequences. Counts chars (not graphemes).
pub(super) fn visible_width(s: &str) -> usize {
    let mut count = 0usize;
    let mut in_esc = false;
    for ch in s.chars() {
        if in_esc {
            if ch == 'm' { in_esc = false; }
            continue;
        }
        if ch == '\x1b' { in_esc = true; continue; }
        count += 1;
    }
    count
}

/// Truncate a string that may contain ANSI escapes to a visible width budget.
/// Appends `…` if truncated. Resets style at end so we don't bleed.
pub(super) fn truncate_visible(s: &str, max: usize) -> String {
    if visible_width(s) <= max { return s.to_string(); }
    let mut out = String::new();
    let mut visible = 0usize;
    let mut in_esc = false;
    for ch in s.chars() {
        if in_esc {
            out.push(ch);
            if ch == 'm' { in_esc = false; }
            continue;
        }
        if ch == '\x1b' { in_esc = true; out.push(ch); continue; }
        if visible + 1 > max.saturating_sub(1) { break; }
        out.push(ch);
        visible += 1;
    }
    out.push('…');
    out.push_str("\x1b[0m");
    out
}

pub(super) fn truncate_chars(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max { return s.to_string(); }
    let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_width_ignores_escapes() {
        let painted = "\x1b[32mhello\x1b[0m"; // 5 visible chars + escapes
        assert_eq!(visible_width(painted), 5);
    }

    #[test]
    fn truncate_visible_respects_budget_with_escapes() {
        let painted = "\x1b[32mabcdefghij\x1b[0m"; // 10 visible
        let out = truncate_visible(painted, 5);
        assert_eq!(visible_width(&out), 5);
    }

    #[test]
    fn truncate_visible_passes_through_when_under_budget() {
        let out = truncate_visible("hi", 10);
        assert_eq!(out, "hi");
    }

    #[test]
    fn truncate_chars_appends_ellipsis_when_over_budget() {
        let out = truncate_chars("abcdef", 4);
        assert_eq!(out, "abc…");
    }

    #[test]
    fn truncate_chars_passes_through_when_under_budget() {
        let out = truncate_chars("abc", 4);
        assert_eq!(out, "abc");
    }
}
