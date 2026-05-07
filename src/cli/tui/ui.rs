//! Pure render function. Takes &App, draws into a Frame. No I/O, no
//! state mutation.

use super::app::{App, Mode, Tab};
use crate::probe::ProbeKind;
use crate::state::PortCard;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

const TABS: &[Tab] = &[Tab::Live, Tab::All, Tab::Error, Tab::Dead];

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(1),    // list
            Constraint::Length(1), // footer
        ])
        .split(area);

    render_header(f, chunks[0], app);
    render_list(f, chunks[1], app);
    render_footer(f, chunks[2], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let counts = app.counts();
    let mut spans = vec![
        Span::styled(
            "portbook",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    for &t in TABS {
        let count = match t {
            Tab::Live => counts.live,
            Tab::All => counts.total,
            Tab::Error => counts.error,
            Tab::Dead => counts.dead,
        };
        let label = format!("{}({}) ", t.label(), count);
        let style = if t == app.tab {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(label, style));
    }
    if !app.filter.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled("filter:", Style::default().fg(Color::DarkGray)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            app.filter.clone(),
            Style::default().fg(Color::Yellow),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_list(f: &mut Frame, area: Rect, app: &App) {
    let visible = app.visible_indices();
    if visible.is_empty() {
        let msg = if app.snapshot.ports.is_empty() {
            "no listeners detected yet…"
        } else if !app.filter.is_empty() {
            "no rows match filter"
        } else {
            "no rows in this tab"
        };
        f.render_widget(
            Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)),
            area,
        );
        return;
    }

    let width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&i| build_item(&app.snapshot.ports[i], app.expanded.contains(&app.snapshot.ports[i].port), width))
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn build_item(card: &PortCard, expanded: bool, width: usize) -> ListItem<'static> {
    let kind_color = if card.is_pending() {
        Color::DarkGray
    } else {
        match card.kind {
            ProbeKind::Live => Color::Green,
            ProbeKind::Error => Color::Yellow,
            ProbeKind::Dead => Color::Red,
        }
    };
    let port = format!(":{}", card.port);
    let title = card.title.as_deref().unwrap_or("(no title)");
    let project = card.project_name.as_deref().unwrap_or("");
    let mut head: Vec<Span> = vec![
        Span::styled(
            format!("  {:<7}", port),
            Style::default().fg(kind_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::raw(truncate(title.to_string(), width.saturating_sub(20))),
    ];
    if !project.is_empty() {
        head.push(Span::raw("  "));
        head.push(Span::styled(
            format!("[{project}]"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let show_reason = card.is_pending() || !matches!(card.kind, ProbeKind::Live);
    if show_reason
        && let Some(reason) = card.reason.as_deref()
    {
        head.push(Span::raw("  "));
        head.push(Span::styled(
            reason.to_string(),
            Style::default().fg(kind_color),
        ));
    }

    let mut lines: Vec<Line> = vec![Line::from(head)];
    if expanded {
        lines.extend(detail_lines(card));
    }
    ListItem::new(lines)
}

fn detail_lines(card: &PortCard) -> Vec<Line<'static>> {
    let detail = |k: &str, v: String| {
        Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{k:<13}"), Style::default().fg(Color::DarkGray)),
            Span::raw(v),
        ])
    };
    let mut out = vec![
        detail("url", card.url.clone()),
        detail(
            "kind/probe",
            format!(
                "{} · {} ms · attempts={}",
                kind_str(card.kind),
                card.elapsed_ms.unwrap_or(0),
                card.attempts
            ),
        ),
    ];
    if let Some(s) = card.status {
        out.push(detail("http status", s.to_string()));
    }
    if let Some(e) = card.error_class {
        let cls = serde_json::to_value(e)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        out.push(detail("error class", cls));
    }
    if let Some(d) = card.error_detail.as_deref() {
        out.push(detail("error detail", d.to_string()));
    }
    if let Some(d) = card.description.as_deref() {
        out.push(detail("description", d.to_string()));
    }
    if let Some(d) = card.cwd.as_deref() {
        out.push(detail("cwd", d.to_string()));
    }
    if let Some(d) = card.cmdline.as_deref() {
        out.push(detail("cmdline", d.to_string()));
    }
    out.push(detail("pid", card.pid.to_string()));
    out
}

fn kind_str(k: ProbeKind) -> &'static str {
    match k {
        ProbeKind::Live => "live",
        ProbeKind::Error => "error",
        ProbeKind::Dead => "dead",
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let scan_label = match app.snapshot.scan_elapsed_ms {
        Some(ms) => format!("scan:{ms}ms"),
        None if !app.snapshot.ports.is_empty() => "scan:probing…".to_string(),
        None => "scan:—".to_string(),
    };
    let hint = match app.mode {
        Mode::Normal => format!(
            "j/k · Tab · Space expand · Enter open · / filter · q quit  ·  src:{} · {}  v{}",
            app.source_label,
            scan_label,
            env!("CARGO_PKG_VERSION")
        ),
        Mode::Filtering => format!(
            "filter: {}_   Enter accept · Esc cancel",
            app.filter
        ),
    };
    let mut spans = vec![Span::styled(hint, Style::default().fg(Color::DarkGray))];
    if let Some(msg) = app.status_message() {
        spans = vec![Span::styled(
            msg.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )];
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn truncate(s: String, max: usize) -> String {
    if s.chars().count() <= max || max == 0 {
        s
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
