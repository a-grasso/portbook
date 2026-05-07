//! TUI state machine. Pure — no I/O, no rendering. Unit-testable.

use crate::probe::ProbeKind;
use crate::state::{PortCard, Snapshot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashSet;
use std::time::Instant;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Live,
    All,
    Error,
    Dead,
}

impl Tab {
    pub fn next(self) -> Self {
        match self {
            Tab::Live => Tab::All,
            Tab::All => Tab::Error,
            Tab::Error => Tab::Dead,
            Tab::Dead => Tab::Live,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Tab::Live => "Live",
            Tab::All => "All",
            Tab::Error => "Error",
            Tab::Dead => "Dead",
        }
    }
    pub fn matches(self, k: ProbeKind) -> bool {
        match self {
            Tab::All => true,
            Tab::Live => k == ProbeKind::Live,
            Tab::Error => k == ProbeKind::Error,
            Tab::Dead => k == ProbeKind::Dead,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Filtering,
}

pub struct App {
    pub snapshot: Snapshot,
    pub tab: Tab,
    pub selected: usize,
    pub expanded: HashSet<u16>,
    pub filter: String,
    pub mode: Mode,
    pub should_quit: bool,
    pub source_label: &'static str,
    /// Transient one-line message shown in the footer (e.g. "opened 3000
    /// in browser"). Decays after `STATUS_TTL`.
    pub status: Option<(String, Instant)>,
}

const STATUS_TTL: std::time::Duration = std::time::Duration::from_millis(2500);

impl App {
    pub fn new(source_label: &'static str) -> Self {
        Self {
            snapshot: Snapshot { ports: Vec::new() },
            tab: Tab::default(),
            selected: 0,
            expanded: HashSet::new(),
            filter: String::new(),
            mode: Mode::default(),
            should_quit: false,
            source_label,
            status: None,
        }
    }

    /// Live status line, accounting for TTL.
    pub fn status_message(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, at)| at.elapsed() < STATUS_TTL)
            .map(|(s, _)| s.as_str())
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    /// Drop in a fresh snapshot. Preserves the user's selection by port
    /// where possible — if the previously selected port is still listed,
    /// the cursor stays on it. Otherwise it clamps.
    pub fn ingest(&mut self, snap: Snapshot) {
        let prev_port = self.visible_indices_for(&snap, &self.snapshot.ports)
            .first()
            .copied();
        // Compute previous selection target before replacing snapshot.
        let prev_selected_port = self.selected_port();
        self.snapshot = snap;
        let visible = self.visible_indices();
        if let Some(port) = prev_selected_port
            && let Some(pos) = visible
                .iter()
                .position(|&i| self.snapshot.ports[i].port == port)
        {
            self.selected = pos;
            return;
        }
        let _ = prev_port;
        if visible.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(visible.len() - 1);
        }
    }

    /// Indices into `self.snapshot.ports` that match the current tab + filter.
    pub fn visible_indices(&self) -> Vec<usize> {
        let f = self.filter.to_lowercase();
        self.snapshot
            .ports
            .iter()
            .enumerate()
            .filter(|(_, c)| self.tab.matches(c.kind))
            .filter(|(_, c)| f.is_empty() || row_matches(c, &f))
            .map(|(i, _)| i)
            .collect()
    }

    fn visible_indices_for(&self, _snap: &Snapshot, _prev: &[PortCard]) -> Vec<usize> {
        // Reserved for future smarter diff. Currently unused.
        Vec::new()
    }

    pub fn selected_card(&self) -> Option<&PortCard> {
        let idxs = self.visible_indices();
        idxs.get(self.selected)
            .and_then(|&i| self.snapshot.ports.get(i))
    }

    pub fn selected_port(&self) -> Option<u16> {
        self.selected_card().map(|c| c.port)
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for p in &self.snapshot.ports {
            match p.kind {
                ProbeKind::Live => c.live += 1,
                ProbeKind::Error => c.error += 1,
                ProbeKind::Dead => c.dead += 1,
            }
            c.total += 1;
        }
        c
    }

    /// Apply a key. Mode-aware: filtering captures most printable input.
    pub fn handle_key(&mut self, k: KeyEvent) {
        if self.mode == Mode::Filtering {
            self.handle_key_filtering(k);
        } else {
            self.handle_key_normal(k);
        }
    }

    fn handle_key_normal(&mut self, k: KeyEvent) {
        // Ctrl-C always quits.
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        match k.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Tab => {
                self.tab = self.tab.next();
                self.selected = 0;
            }
            KeyCode::Char('/') => {
                self.mode = Mode::Filtering;
                self.set_status("filter: type, Enter to apply, Esc to cancel");
            }
            KeyCode::Char(' ') | KeyCode::Char('x') => self.toggle_expand(),
            KeyCode::Enter => self.handle_enter(),
            KeyCode::Esc => {
                if !self.filter.is_empty() {
                    self.filter.clear();
                    self.set_status("filter cleared");
                }
            }
            _ => {}
        }
    }

    fn handle_key_filtering(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.mode = Mode::Normal;
                self.selected = 0;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
            }
            _ => {}
        }
    }

    fn handle_enter(&mut self) {
        let card = match self.selected_card() {
            Some(c) => c.clone(),
            None => return,
        };
        if !matches!(card.kind, ProbeKind::Live) {
            self.set_status(format!(
                "port {} is {} — only live ports can be opened",
                card.port,
                kind_label(card.kind)
            ));
            return;
        }
        if super::open_in_browser(&card.url) {
            self.set_status(format!("opened {} in browser", card.url));
        } else {
            self.set_status(format!("failed to open {}", card.url));
        }
    }

    fn toggle_expand(&mut self) {
        if let Some(port) = self.selected_port()
            && !self.expanded.insert(port)
        {
            self.expanded.remove(&port);
        }
    }

    fn move_selection(&mut self, delta: i32) {
        let len = self.visible_indices().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        let cur = self.selected as i32;
        let new = (cur + delta).clamp(0, len as i32 - 1);
        self.selected = new as usize;
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub live: usize,
    pub error: usize,
    pub dead: usize,
    pub total: usize,
}

fn kind_label(k: ProbeKind) -> &'static str {
    match k {
        ProbeKind::Live => "live",
        ProbeKind::Error => "error",
        ProbeKind::Dead => "dead",
    }
}

fn row_matches(c: &PortCard, needle: &str) -> bool {
    let port_s = c.port.to_string();
    let mut hay: Vec<&str> = vec![&port_s, &c.command];
    if let Some(s) = c.title.as_deref() {
        hay.push(s);
    }
    if let Some(s) = c.project_name.as_deref() {
        hay.push(s);
    }
    if let Some(s) = c.cmdline.as_deref() {
        hay.push(s);
    }
    if let Some(s) = c.cwd.as_deref() {
        hay.push(s);
    }
    hay.iter().any(|s| s.to_lowercase().contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::ProbeResult;
    use crate::process::ProcInfo;

    fn card(port: u16, kind: ProbeKind, title: &str) -> PortCard {
        let probe = ProbeResult {
            kind,
            status: Some(200),
            title: Some(title.into()),
            description: None,
            reason: None,
            probed_url: format!("http://127.0.0.1:{port}/"),
            probed_at_unix: 0,
            elapsed_ms: 12,
            error_class: None,
            error_detail: None,
            attempts: 1,
        };
        let proc = ProcInfo {
            cwd: Some("/tmp/sample".into()),
            cmdline: Some(format!("server --port {port}")),
        };
        PortCard::build(port, 1, "server".into(), &proc, &probe)
    }

    fn snap(cards: Vec<PortCard>) -> Snapshot {
        Snapshot { ports: cards }
    }

    #[test]
    fn tab_cycle_visits_all_four() {
        let mut t = Tab::Live;
        let mut seen = vec![t];
        for _ in 0..4 {
            t = t.next();
            seen.push(t);
        }
        assert_eq!(
            seen,
            vec![Tab::Live, Tab::All, Tab::Error, Tab::Dead, Tab::Live]
        );
    }

    #[test]
    fn live_tab_filters_to_live_only() {
        let mut a = App::new("test");
        a.ingest(snap(vec![
            card(3000, ProbeKind::Live, "ok"),
            card(3001, ProbeKind::Dead, "down"),
            card(3002, ProbeKind::Error, "404"),
        ]));
        assert_eq!(a.visible_indices().len(), 1);
        assert_eq!(a.selected_card().map(|c| c.port), Some(3000));
    }

    #[test]
    fn filter_matches_across_fields() {
        let mut a = App::new("test");
        a.tab = Tab::All;
        a.ingest(snap(vec![
            card(3000, ProbeKind::Live, "Frontend"),
            card(8080, ProbeKind::Live, "Metrics"),
        ]));
        a.filter = "metric".into();
        assert_eq!(a.visible_indices().len(), 1);
        assert_eq!(a.selected_card().map(|c| c.port), Some(8080));
    }

    #[test]
    fn ingest_preserves_selection_by_port() {
        let mut a = App::new("test");
        a.tab = Tab::All;
        a.ingest(snap(vec![
            card(3000, ProbeKind::Live, "a"),
            card(3001, ProbeKind::Live, "b"),
            card(3002, ProbeKind::Live, "c"),
        ]));
        a.selected = 1;
        // Snapshot reordered + a new port appears — selection should still
        // ride 3001.
        a.ingest(snap(vec![
            card(2000, ProbeKind::Live, "new"),
            card(3000, ProbeKind::Live, "a"),
            card(3001, ProbeKind::Live, "b"),
            card(3002, ProbeKind::Live, "c"),
        ]));
        assert_eq!(a.selected_card().map(|c| c.port), Some(3001));
    }

    #[test]
    fn navigation_clamps_to_visible_range() {
        let mut a = App::new("test");
        a.tab = Tab::All;
        a.ingest(snap(vec![
            card(3000, ProbeKind::Live, "a"),
            card(3001, ProbeKind::Live, "b"),
        ]));
        a.move_selection(99);
        assert_eq!(a.selected, 1);
        a.move_selection(-99);
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn slash_enters_filter_mode_then_esc_clears() {
        let mut a = App::new("test");
        a.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(a.mode, Mode::Filtering);
        a.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(a.filter, "x");
        a.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(a.mode, Mode::Normal);
        assert_eq!(a.filter, "");
    }

    #[test]
    fn ctrl_c_quits_in_normal_mode() {
        let mut a = App::new("test");
        a.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(a.should_quit);
    }

    #[test]
    fn space_toggles_expand_for_selected_row() {
        let mut a = App::new("test");
        a.tab = Tab::All;
        a.ingest(snap(vec![card(3000, ProbeKind::Live, "x")]));
        a.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(a.expanded.contains(&3000));
        a.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(!a.expanded.contains(&3000));
    }
}
