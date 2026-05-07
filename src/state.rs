use crate::probe::{ProbeError, ProbeKind, ProbeResult};
use crate::process::ProcInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortCard {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    pub url: String,
    pub kind: ProbeKind,
    pub reason: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub project_root: Option<String>,
    pub project_name: Option<String>,
    pub cwd: Option<String>,
    pub cmdline: Option<String>,
    pub status: Option<u16>,
    pub probed_url: Option<String>,
    pub probed_at_unix: Option<u64>,
    pub elapsed_ms: Option<u32>,
    pub error_class: Option<ProbeError>,
    pub error_detail: Option<String>,
    pub attempts: u8,
}

impl PortCard {
    pub fn build(port: u16, pid: u32, command: String, proc: &ProcInfo, probe: &ProbeResult) -> Self {
        let project_root = proc.cwd.as_deref().and_then(crate::project::detect_root);
        let project_name = project_root.as_deref().map(crate::project::folder_name);
        Self {
            port,
            pid,
            command,
            url: format!("http://localhost:{port}"),
            kind: probe.kind,
            reason: probe.reason.clone(),
            title: probe.title.clone(),
            description: probe.description.clone(),
            project_root,
            project_name,
            cwd: proc.cwd.clone(),
            cmdline: proc.cmdline.as_deref().map(crate::redact::redact_cmdline),
            status: probe.status,
            probed_url: Some(probe.probed_url.clone()),
            probed_at_unix: Some(probe.probed_at_unix),
            elapsed_ms: Some(probe.elapsed_ms),
            error_class: probe.error_class,
            error_detail: probe.error_detail.clone(),
            attempts: probe.attempts,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<RwLock<Inner>>,
    tx: broadcast::Sender<Snapshot>,
}

#[derive(Default)]
struct Inner {
    cards: HashMap<u16, PortCard>,
    /// Wall time of the most recent scan that produced these cards.
    /// `None` means the cards are a skeleton (enumeration only, probes
    /// in flight) — not yet a full scan result.
    scan_elapsed_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub ports: Vec<PortCard>,
    /// Wall time of the scan cycle that produced this snapshot, in
    /// milliseconds. `None` for skeleton snapshots (enumerate-only,
    /// probes still in flight) and for pre-v0.1.7 snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_elapsed_ms: Option<u32>,
}

/// Marker reason set on a [`PortCard`] when the row was synthesized
/// from enumeration only — its real probe is still in flight. The TUI
/// renders these as neutral/pending and includes them in the Live tab.
pub const PENDING_REASON: &str = "probing…";

impl PortCard {
    /// Build a placeholder card from enumeration + process inspection
    /// only — no probe has happened yet. Used to paint a skeleton on
    /// first frame so users don't stare at an empty UI for ~5s while
    /// retries time out.
    pub fn pending(port: u16, pid: u32, command: String, proc: &ProcInfo) -> Self {
        let project_root = proc.cwd.as_deref().and_then(crate::project::detect_root);
        let project_name = project_root.as_deref().map(crate::project::folder_name);
        Self {
            port,
            pid,
            command,
            url: format!("http://localhost:{port}"),
            kind: ProbeKind::Dead,
            reason: Some(PENDING_REASON.into()),
            title: None,
            description: None,
            project_root,
            project_name,
            cwd: proc.cwd.clone(),
            cmdline: proc.cmdline.as_deref().map(crate::redact::redact_cmdline),
            status: None,
            probed_url: None,
            probed_at_unix: None,
            elapsed_ms: None,
            error_class: None,
            error_detail: None,
            attempts: 0,
        }
    }

    /// True when this card is a skeleton placeholder rather than a
    /// fully-probed result. See [`PortCard::pending`].
    pub fn is_pending(&self) -> bool {
        self.attempts == 0 && self.reason.as_deref() == Some(PENDING_REASON)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self {
            inner: Arc::new(RwLock::new(Inner::default())),
            tx,
        }
    }

    pub async fn snapshot(&self) -> Snapshot {
        let inner = self.inner.read().await;
        let mut ports: Vec<PortCard> = inner.cards.values().cloned().collect();
        ports.sort_by_key(|c| c.port);
        Snapshot {
            ports,
            scan_elapsed_ms: inner.scan_elapsed_ms,
        }
    }

    /// Replace the full state with a completed scan cycle. `elapsed_ms`
    /// is the wall time of the scan that produced these cards; it
    /// rides along on every emitted [`Snapshot`].
    pub async fn replace(&self, new_map: HashMap<u16, PortCard>, elapsed_ms: Option<u32>) {
        {
            let mut w = self.inner.write().await;
            w.cards = new_map;
            w.scan_elapsed_ms = elapsed_ms;
        }
        let snap = self.snapshot().await;
        let _ = self.tx.send(snap);
    }

    /// Replace the state with skeleton cards (enumeration only, probes
    /// pending). `scan_elapsed_ms` is cleared. Used by the scheduler
    /// to paint a fast first frame before slow probes finish.
    pub async fn replace_skeleton(&self, skeleton: HashMap<u16, PortCard>) {
        self.replace(skeleton, None).await;
    }

    /// Insert or replace a single card and broadcast the resulting
    /// snapshot. Keeps `scan_elapsed_ms = None` (skeleton state); the
    /// caller marks the cycle done with [`AppState::mark_done`].
    ///
    /// Used by the scheduler's per-probe streaming path to avoid
    /// cloning the entire port map on every probe completion.
    pub async fn update_one(&self, card: PortCard) {
        {
            let mut w = self.inner.write().await;
            w.cards.insert(card.port, card);
            w.scan_elapsed_ms = None;
        }
        let snap = self.snapshot().await;
        let _ = self.tx.send(snap);
    }

    /// Stamp the current state as a resolved cycle and broadcast.
    /// Called once per cycle after all per-probe `update_one` calls.
    pub async fn mark_done(&self, elapsed_ms: u32) {
        {
            let mut w = self.inner.write().await;
            w.scan_elapsed_ms = Some(elapsed_ms);
        }
        let snap = self.snapshot().await;
        let _ = self.tx.send(snap);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Snapshot> {
        self.tx.subscribe()
    }
}
