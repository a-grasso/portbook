//! Snapshot rendering — both human (grouped, colored) and JSON modes.
//! Writes into any `impl Write` so it's testable.

use super::LsOpts;
use super::style::Style;
use super::width::{truncate_chars, truncate_visible, visible_width};
use crate::probe::ProbeKind;
use crate::state::{PortCard, Snapshot};
use std::io::Write;

pub(super) fn render(
    out: &mut impl Write,
    snap: &Snapshot,
    opts: LsOpts,
    s: &Style,
    width: usize,
) -> std::io::Result<()> {
    if opts.json {
        let line = serde_json::to_string(snap).map_err(std::io::Error::other)?;
        writeln!(out, "{}", line)?;
        return Ok(());
    }

    let total = snap.ports.len();
    let live: Vec<&PortCard> = snap.ports.iter().filter(|c| c.kind == ProbeKind::Live).collect();
    let errors: Vec<&PortCard> = snap.ports.iter().filter(|c| c.kind == ProbeKind::Error).collect();
    let dead: Vec<&PortCard> = snap.ports.iter().filter(|c| c.kind == ProbeKind::Dead).collect();

    writeln!(
        out,
        "{} · {} live · {} total\n",
        s.bold("portbook"),
        s.green(&live.len().to_string()),
        total,
    )?;

    if total == 0 {
        writeln!(out, "  No listening ports detected.")?;
        return Ok(());
    }

    write_section(out, "LIVE", '●', &live, opts, s, width)?;
    if !opts.live {
        write_section(out, "ERROR", '⚠', &errors, opts, s, width)?;
        if opts.all {
            write_section(out, "DEAD", '·', &dead, opts, s, width)?;
        } else if !dead.is_empty() {
            writeln!(
                out,
                "  {}  {} dead {}",
                s.dim("·"),
                s.dim(&dead.len().to_string()),
                s.dim("(pass --all to expand)"),
            )?;
            writeln!(out)?;
        }
    }
    Ok(())
}

fn write_section(
    out: &mut impl Write,
    name: &str,
    glyph: char,
    cards: &[&PortCard],
    _opts: LsOpts,
    s: &Style,
    width: usize,
) -> std::io::Result<()> {
    if cards.is_empty() {
        return Ok(());
    }
    writeln!(out, " {}", s.bold(name))?;

    let mut sorted: Vec<&PortCard> = cards.to_vec();
    sorted.sort_by(|a, b| {
        a.project_name
            .as_deref()
            .unwrap_or("\u{FFFF}")
            .cmp(b.project_name.as_deref().unwrap_or("\u{FFFF}"))
            .then(a.port.cmp(&b.port))
    });

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
        write_card(out, c, glyph, s, width, port_w, project_w)?;
    }
    writeln!(out)?;
    Ok(())
}

fn write_card(
    out: &mut impl Write,
    c: &PortCard,
    glyph: char,
    s: &Style,
    width: usize,
    port_w: usize,
    project_w: usize,
) -> std::io::Result<()> {
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
    writeln!(out, "{}{}{}", head, body, tail)?;

    let cmd = c.cmdline.clone().unwrap_or_else(|| c.command.clone());
    let cwd_short = c.cwd_short.as_deref().filter(|s| !s.is_empty()).map(str::to_owned);
    let sub = match (cwd_short, cmd.is_empty()) {
        (Some(p), false) => format!("{p}  ·  {cmd}"),
        (Some(p), true) => p,
        (None, false) => cmd,
        (None, true) => String::new(),
    };
    if !sub.is_empty() {
        let indent = "      ";
        let budget = width.saturating_sub(indent.chars().count()).max(20);
        let truncated = truncate_chars(&sub, budget);
        writeln!(out, "{}{}", indent, s.dim(&truncated))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::ProbeResult;
    use crate::process::ProcInfo;

    fn fixture_snapshot() -> Snapshot {
        let probe_live = ProbeResult {
            kind: ProbeKind::Live,
            status: Some(200),
            title: Some("Hello".into()),
            description: None,
            reason: None,
            probed_url: "http://127.0.0.1:8000/".into(),
            probed_at_unix: 0,
            elapsed_ms: 12,
            error_class: None,
            error_detail: None,
            attempts: 1,
        };
        let proc = ProcInfo { cwd: None, cmdline: Some("python -m http.server".into()) };
        Snapshot {
            ports: vec![PortCard::build(8000, 1234, "python".into(), &proc, &probe_live)],
            scan_elapsed_ms: None,
        }
    }

    #[test]
    fn json_mode_emits_parseable_snapshot_with_ports_array() {
        let snap = fixture_snapshot();
        let opts = LsOpts { json: true, ..Default::default() };
        let style = Style { enabled: false };
        let mut buf: Vec<u8> = Vec::new();
        render(&mut buf, &snap, opts, &style, 120).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&buf)
            .expect("output must be valid JSON in --json mode");
        let ports = parsed.get("ports").expect("must have ports field");
        assert_eq!(ports.as_array().unwrap().len(), 1);
        assert_eq!(ports[0]["port"], 8000);
        assert_eq!(ports[0]["kind"], "live");
    }

    #[test]
    fn json_mode_contains_no_ansi_escapes() {
        let snap = fixture_snapshot();
        let opts = LsOpts { json: true, ..Default::default() };
        let style = Style { enabled: true };
        let mut buf: Vec<u8> = Vec::new();
        render(&mut buf, &snap, opts, &style, 120).unwrap();
        assert!(!buf.contains(&b'\x1b'), "JSON output must contain no ANSI escapes");
    }

    #[test]
    fn json_mode_ends_with_newline() {
        let snap = fixture_snapshot();
        let opts = LsOpts { json: true, ..Default::default() };
        let style = Style { enabled: false };
        let mut buf: Vec<u8> = Vec::new();
        render(&mut buf, &snap, opts, &style, 120).unwrap();
        assert_eq!(buf.last(), Some(&b'\n'), "JSON line must end with newline for stream consumers");
    }

    #[test]
    fn human_mode_writes_to_provided_writer() {
        let snap = fixture_snapshot();
        let opts = LsOpts::default();
        let style = Style { enabled: false };
        let mut buf: Vec<u8> = Vec::new();
        render(&mut buf, &snap, opts, &style, 120).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("portbook"), "human mode should print the header");
        assert!(out.contains(":8000"), "human mode should print the port");
    }
}
