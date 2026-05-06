//! Terminal view over the Engine. View-only — no discovery / probe logic
//! here, see `ARCHITECTURE.md`.

use crate::BIND_ADDR;
use crate::engine::Engine;
use crate::probe::ProbeKind;
use crate::state::{PortCard, Snapshot};
use std::io::IsTerminal;

#[derive(Default, Debug, Clone, Copy)]
pub struct LsOpts {
    pub all: bool,
    pub live: bool,
    pub no_color: bool,
}

pub async fn run_ls(opts: LsOpts) -> anyhow::Result<()> {
    let snapshot = match fetch_from_daemon().await {
        Some(s) => s,
        None => one_shot_scan().await?,
    };
    let style = Style::resolve(opts.no_color);
    let width = term_width();
    print_snapshot(&snapshot, opts, &style, width);
    Ok(())
}

async fn fetch_from_daemon() -> Option<Snapshot> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .ok()?;
    let url = format!("http://{BIND_ADDR}/api/ports");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Snapshot>().await.ok()
}

async fn one_shot_scan() -> anyhow::Result<Snapshot> {
    let ports = Engine::new().scan_all().await?;
    Ok(Snapshot { ports })
}

// ─── Rendering ───────────────────────────────────────────────────────

fn print_snapshot(snap: &Snapshot, opts: LsOpts, s: &Style, width: usize) {
    let total = snap.ports.len();
    let live: Vec<&PortCard> = snap.ports.iter().filter(|c| c.kind == ProbeKind::Live).collect();
    let errors: Vec<&PortCard> = snap.ports.iter().filter(|c| c.kind == ProbeKind::Error).collect();
    let dead: Vec<&PortCard> = snap.ports.iter().filter(|c| c.kind == ProbeKind::Dead).collect();

    println!(
        "{} · {} live · {} total\n",
        s.bold("portbook"),
        s.green(&live.len().to_string()),
        total,
    );

    if total == 0 {
        println!("  No listening ports detected.");
        return;
    }

    print_section("LIVE", '●', &live, opts, s, width, false);
    if !opts.live {
        print_section("ERROR", '⚠', &errors, opts, s, width, false);
        if opts.all {
            print_section("DEAD", '·', &dead, opts, s, width, false);
        } else if !dead.is_empty() {
            println!(
                "  {}  {} dead {}",
                s.dim("·"),
                s.dim(&dead.len().to_string()),
                s.dim("(pass --all to expand)"),
            );
            println!();
        }
    }
}

fn print_section(
    name: &str,
    glyph: char,
    cards: &[&PortCard],
    _opts: LsOpts,
    s: &Style,
    width: usize,
    _: bool,
) {
    if cards.is_empty() {
        return;
    }
    println!(" {}", s.bold(name));

    let mut sorted: Vec<&PortCard> = cards.to_vec();
    sorted.sort_by(|a, b| {
        a.project_name
            .as_deref()
            .unwrap_or("\u{FFFF}") // None sorts last
            .cmp(b.project_name.as_deref().unwrap_or("\u{FFFF}"))
            .then(a.port.cmp(&b.port))
    });

    // Compute column widths so port and project align across all rows in
    // this section (strictly visible widths — no ANSI escapes counted).
    let port_w = sorted
        .iter()
        .map(|c| format!(":{}", c.port).chars().count())
        .max()
        .unwrap_or(0);
    let project_w = sorted
        .iter()
        .map(|c| c.project_name.as_deref().unwrap_or("").chars().count())
        .max()
        .unwrap_or(0);

    for c in sorted {
        print_card(c, glyph, s, width, port_w, project_w);
    }
    println!();
}

fn print_card(c: &PortCard, glyph: char, s: &Style, width: usize, port_w: usize, project_w: usize) {
    let port_raw = format!(":{}", c.port);
    let port_pad = " ".repeat(port_w.saturating_sub(port_raw.chars().count()));
    let glyph_colored = match c.kind {
        ProbeKind::Live => s.green(&glyph.to_string()),
        ProbeKind::Error => s.amber(&glyph.to_string()),
        ProbeKind::Dead => s.dim(&glyph.to_string()),
    };
    let project_raw = c.project_name.as_deref().unwrap_or("");
    let project_pad = " ".repeat(project_w.saturating_sub(project_raw.chars().count()));
    let project_col = if project_raw.is_empty() {
        " ".repeat(project_w)
    } else {
        format!("{}{}", s.cyan(project_raw), project_pad)
    };
    let title_or_reason = match c.kind {
        ProbeKind::Live => c.title.clone().unwrap_or_else(|| "(no title)".into()),
        _ => c.reason.clone().unwrap_or_else(|| format!("{:?}", c.kind).to_lowercase()),
    };
    let url = match c.kind {
        ProbeKind::Live => s.url(&c.url),
        _ => s.dim(&c.url),
    };

    let arrow = s.dim("→");
    let head = format!(
        " {}  {}{}  {}{}  ",
        glyph_colored,
        s.port(&port_raw),
        port_pad,
        if project_w > 0 { project_col } else { String::new() },
        if project_w > 0 { "  " } else { "" },
    );
    let tail = format!("  {}  {}", arrow, url);

    let head_vis = visible_width(&head);
    let tail_vis = visible_width(&tail);
    let body_budget = width.saturating_sub(head_vis + tail_vis).max(20);

    let body = truncate_visible(&title_or_reason, body_budget);
    println!("{}{}{}", head, body, tail);

    // Line 2: indented dim cmdline
    let cmd = c.cmdline.clone().unwrap_or_else(|| c.command.clone());
    if !cmd.is_empty() {
        let indent = "      ";
        let cmd_budget = width.saturating_sub(indent.chars().count()).max(20);
        let cmd_truncated = truncate_chars(&cmd, cmd_budget);
        println!("{}{}", indent, s.dim(&cmd_truncated));
    }
}

// ─── Style helpers (anstyle-based, conditional) ──────────────────────

use anstyle::{AnsiColor, Effects, Style as AStyle};

#[derive(Default)]
pub struct Style {
    enabled: bool,
}

impl Style {
    pub fn resolve(force_off: bool) -> Self {
        if force_off {
            return Self { enabled: false };
        }
        if std::env::var_os("NO_COLOR").is_some() {
            return Self { enabled: false };
        }
        Self { enabled: std::io::stdout().is_terminal() }
    }

    fn paint(&self, style: AStyle, s: &str) -> String {
        if self.enabled {
            format!("{style}{s}{style:#}")
        } else {
            s.to_string()
        }
    }

    pub fn bold(&self, s: &str) -> String {
        self.paint(AStyle::new().effects(Effects::BOLD), s)
    }
    pub fn dim(&self, s: &str) -> String {
        self.paint(AStyle::new().effects(Effects::DIMMED), s)
    }
    pub fn green(&self, s: &str) -> String {
        self.paint(AStyle::new().fg_color(Some(AnsiColor::Green.into())), s)
    }
    pub fn amber(&self, s: &str) -> String {
        self.paint(AStyle::new().fg_color(Some(AnsiColor::Yellow.into())), s)
    }
    pub fn cyan(&self, s: &str) -> String {
        self.paint(AStyle::new().fg_color(Some(AnsiColor::Cyan.into())), s)
    }
    pub fn port(&self, s: &str) -> String {
        self.paint(
            AStyle::new()
                .fg_color(Some(AnsiColor::BrightBlue.into()))
                .effects(Effects::BOLD),
            s,
        )
    }
    pub fn url(&self, s: &str) -> String {
        self.paint(
            AStyle::new()
                .fg_color(Some(AnsiColor::Blue.into()))
                .effects(Effects::UNDERLINE),
            s,
        )
    }
}

// ─── Width helpers ───────────────────────────────────────────────────

fn term_width() -> usize {
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
fn visible_width(s: &str) -> usize {
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
fn truncate_visible(s: &str, max: usize) -> String {
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

fn truncate_chars(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max { return s.to_string(); }
    let mut out: String = chars.into_iter().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_disabled_emits_no_escapes() {
        let s = Style { enabled: false };
        assert_eq!(s.green("ok"), "ok");
        assert_eq!(s.bold("hi"), "hi");
    }

    #[test]
    fn style_enabled_wraps_with_escapes() {
        let s = Style { enabled: true };
        let out = s.green("ok");
        assert!(out.starts_with("\x1b[32m"));
        assert!(out.ends_with("\x1b[0m"));
    }

    #[test]
    fn visible_width_ignores_escapes() {
        let s = Style { enabled: true };
        let painted = s.green("hello"); // 5 visible chars + escapes
        assert_eq!(visible_width(&painted), 5);
    }

    #[test]
    fn truncate_visible_respects_budget_with_escapes() {
        let s = Style { enabled: true };
        let painted = s.green("abcdefghij"); // 10 visible
        let out = truncate_visible(&painted, 5);
        // visible width should be 5 (4 chars + ellipsis)
        assert_eq!(visible_width(&out), 5);
    }

    #[test]
    fn truncate_visible_passes_through_when_under_budget() {
        let out = truncate_visible("hi", 10);
        assert_eq!(out, "hi");
    }
}
