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
    /// Abbreviated `cwd` (parents collapsed to single chars, leaf intact, `$HOME` → `~`).
    /// Computed on the daemon so consumers don't need to know the server's HOME.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd_short: Option<String>,
    pub cmdline: Option<String>,
    pub status: Option<u16>,
    pub probed_url: Option<String>,
    pub probed_at_unix: Option<u64>,
    pub elapsed_ms: Option<u32>,
    pub error_class: Option<ProbeError>,
    pub error_detail: Option<String>,
    pub attempts: u8,
    /// True when this card is a skeleton placeholder (enumeration only, probe in flight).
    /// Skipped when false: keeps wire format compact and preserves backward compat with
    /// older consumers that lack the field (they fall back to the `reason`/`attempts` heuristic).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
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
            cwd_short: proc.cwd.as_deref().map(crate::project::shrink_path),
            cmdline: proc.cmdline.as_deref().map(crate::redact::redact_cmdline),
            status: probe.status,
            probed_url: Some(probe.probed_url.clone()),
            probed_at_unix: Some(probe.probed_at_unix),
            elapsed_ms: Some(probe.elapsed_ms),
            error_class: probe.error_class,
            error_detail: probe.error_detail.clone(),
            attempts: probe.attempts,
            pending: false,
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
    /// `None` means cards are a skeleton (enumeration only, probes in flight).
    scan_elapsed_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub ports: Vec<PortCard>,
    /// `None` for skeleton snapshots (probes in flight) and for older snapshots that lack the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_elapsed_ms: Option<u32>,
}

/// Marker reason set on placeholder rows whose real probe is still in flight.
pub(crate) const PENDING_REASON: &str = "probing…";

impl PortCard {
    /// Placeholder card from enumeration only — no probe has happened yet.
    pub(crate) fn pending(port: u16, pid: u32, command: String, proc: &ProcInfo) -> Self {
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
            cwd_short: proc.cwd.as_deref().map(crate::project::shrink_path),
            cmdline: proc.cmdline.as_deref().map(crate::redact::redact_cmdline),
            status: None,
            probed_url: None,
            probed_at_unix: None,
            elapsed_ms: None,
            error_class: None,
            error_detail: None,
            attempts: 0,
            pending: true,
        }
    }

    /// True when this card is a skeleton placeholder rather than a fully-probed result.
    /// Falls back to the `reason`/`attempts` heuristic so older snapshots without the
    /// `pending` field still round-trip correctly.
    pub fn is_pending(&self) -> bool {
        self.pending
            || (self.attempts == 0 && self.reason.as_deref() == Some(PENDING_REASON))
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

    pub async fn replace(&self, new_map: HashMap<u16, PortCard>, elapsed_ms: Option<u32>) {
        {
            let mut w = self.inner.write().await;
            w.cards = new_map;
            w.scan_elapsed_ms = elapsed_ms;
        }
        let snap = self.snapshot().await;
        let _ = self.tx.send(snap);
    }

    /// Replace state with skeleton cards (probes pending); clears `scan_elapsed_ms`.
    pub async fn replace_skeleton(&self, skeleton: HashMap<u16, PortCard>) {
        self.replace(skeleton, None).await;
    }

    /// Insert/replace a single card and broadcast. Keeps `scan_elapsed_ms = None` until
    /// the caller stamps the cycle done via [`AppState::mark_done`].
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

#[cfg(test)]
mod pending_field_tests {
    use super::*;

    #[test]
    fn pending_card_serializes_with_pending_true() {
        let card = PortCard::pending(8080, 1, "x".into(), &ProcInfo::default());
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains("\"pending\":true"), "expected pending:true in {json}");
    }

    #[test]
    fn resolved_card_omits_pending_field() {
        let probe = ProbeResult {
            kind: ProbeKind::Live,
            status: Some(200),
            title: None,
            description: None,
            reason: None,
            probed_url: "http://127.0.0.1:8080/".into(),
            probed_at_unix: 0,
            elapsed_ms: 1,
            error_class: None,
            error_detail: None,
            attempts: 1,
        };
        let card = PortCard::build(8080, 1, "x".into(), &ProcInfo::default(), &probe);
        let json = serde_json::to_string(&card).unwrap();
        assert!(!json.contains("\"pending\""), "pending=false should be skipped: {json}");
    }

    #[test]
    fn legacy_snapshot_without_pending_field_still_recognized() {
        // Pre-v0.1.7 daemons emit the historical "probing…" reason and
        // attempts=0 but no `pending` field. Deserialized into the new
        // struct, is_pending() must still return true so consumers behave
        // correctly during a rolling upgrade.
        let legacy = r#"{
            "port":8080,"pid":1,"command":"x","url":"http://localhost:8080",
            "kind":"dead","reason":"probing…","title":null,"description":null,
            "project_root":null,"project_name":null,"cwd":null,"cmdline":null,
            "status":null,"probed_url":null,"probed_at_unix":null,"elapsed_ms":null,
            "error_class":null,"error_detail":null,"attempts":0
        }"#;
        let card: PortCard = serde_json::from_str(legacy).unwrap();
        assert!(!card.pending, "field defaults to false when missing");
        assert!(card.is_pending(), "legacy reason+attempts heuristic still applies");
    }
}
